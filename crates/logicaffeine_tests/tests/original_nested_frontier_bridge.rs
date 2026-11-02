//! W03 nested inductives across every F01 dimension, each an original Lean 4.14.0 witness.
//!
//! One export per family, each with one user: a universe-polymorphic family, nesting through an
//! INDEXED container, an indexed nested family, dependent and function fields beside nesting,
//! nesting inside an ordinary mutual block, a container with a value parameter, a container with
//! dependent fields, a container with a `Sort`-valued field, nesting under a binder, a
//! proposition nested through `And`, and nesting through a container that is itself nested.
//!
//! Lean's elimination is the reference: its exported recursor records fix how many recursors
//! the block has, their telescope lengths and universes, and the major premise of each (written
//! here by hand from Lean's `#print`). Every user is checked in both kernels with exactly Lean's
//! axiom set (`#print axioms`: none). Each export is then corrupted twice and must be rejected.
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, recheck, BigInt, Context, Literal, Term,
    Universe,
};
use serde_json::Value;

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn g(dotted: &str) -> Term {
    Term::Global(n(dotted))
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
fn at(dotted: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n(dotted),
        levels,
    }
}
fn list0(t: Term) -> Term {
    app(at("List", vec![Universe::Prop]), t)
}
/// Lean's elaborated numeral `@OfNat.ofNat Nat k (instOfNatNat k)`.
fn numeral(k: u64) -> Term {
    let lit = Term::Lit(Literal::Nat(BigInt::from_u64(k)));
    apps(
        at("OfNat.ofNat", vec![Universe::Prop]),
        vec![
            Term::Global("Nat".into()),
            lit.clone(),
            app(g("instOfNatNat"), lit),
        ],
    )
}

/// The major premise Lean prints for one recursor, given the recursor's binders (`b0`, `b1`, …
/// in telescope order) so parameters and indices can be named.
type Major = fn(&[Term]) -> Term;

struct Witness {
    input: &'static str,
    provenance: &'static str,
    source: &'static str,
    source_sha256: &'static str,
    export_sha256: &'static str,
    bytes: usize,
    members: &'static [&'static str],
    user: &'static str,
    /// Recursor names, in Lean's order, with each one's major premise.
    recursors: Vec<(&'static str, Major)>,
}

macro_rules! fixture {
    ($stem:literal, $module:literal) => {
        (
            include_str!(concat!("fixtures/lean_semantics/", $stem, ".ndjson")),
            include_str!(concat!(
                "fixtures/lean_semantics/",
                $stem,
                ".provenance.json"
            )),
            include_str!(concat!("fixtures/lean_semantics/", $module, ".lean")),
        )
    };
}

fn witness(
    files: (&'static str, &'static str, &'static str),
    source_sha256: &'static str,
    export_sha256: &'static str,
    bytes: usize,
    members: &'static [&'static str],
    user: &'static str,
    recursors: Vec<(&'static str, Major)>,
) -> Witness {
    Witness {
        input: files.0,
        provenance: files.1,
        source: files.2,
        source_sha256,
        export_sha256,
        bytes,
        members,
        user,
        recursors,
    }
}

const FRONTIER: &str = "4b33e8b90c8b5af51a736daeea2c0fb4e3c0e68d585328c12576691f3aed8d18";
const SHAPES: &str = "cb65f911d41b3bab15ef0a16918775e7554fc9bd87911e78102cce1fd6879b0a";
const ROSE: &str = "c2218781c5971182c375c19320fd2c9b4dfb31e1d0e3fa9ef8673081b2d1f4db";

fn witnesses() -> Vec<Witness> {
    vec![
        witness(
            fixture!("LF_W03_NestedFrontier.UTree", "LF_W03_NestedFrontier"),
            FRONTIER,
            "88dfd62f4508dd29d89745ff3ad6f2dc15ed9b1ad192e67ae016158c2efaa43e",
            18_251,
            &["LF_W03N.UTree"],
            "LF_W03N.uLeaf",
            vec![
                ("LF_W03N.UTree.rec", |b| {
                    app(
                        Term::Const {
                            name: n("LF_W03N.UTree"),
                            levels: vec![Universe::Var(n("u"))],
                        },
                        b[0].clone(),
                    )
                }),
                ("LF_W03N.UTree.rec_1", |b| {
                    app(
                        Term::Const {
                            name: n("List"),
                            levels: vec![Universe::Var(n("u"))],
                        },
                        app(
                            Term::Const {
                                name: n("LF_W03N.UTree"),
                                levels: vec![Universe::Var(n("u"))],
                            },
                            b[0].clone(),
                        ),
                    )
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_NestedFrontier.VTree", "LF_W03_NestedFrontier"),
            FRONTIER,
            "f5a126b6b893972031258bbfe495bcfa80d8cd892e8950ce40df876dfaa36d39",
            44_478,
            &["LF_W03N.VTree"],
            "LF_W03N.vPair",
            vec![
                ("LF_W03N.VTree.rec", |_| g("LF_W03N.VTree")),
                // The index is the last binder before the major premise — never the literal 2.
                ("LF_W03N.VTree.rec_1", |b| {
                    apps(
                        g("LF_W03N.Vec"),
                        vec![g("LF_W03N.VTree"), b[b.len() - 1].clone()],
                    )
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_NestedFrontier.ITree", "LF_W03_NestedFrontier"),
            FRONTIER,
            "7a3781fc5c73004fed3264b8db1c02d3da4fda50af40fce3d3a156840cc2f613",
            19_041,
            &["LF_W03N.ITree"],
            "LF_W03N.iNode",
            vec![
                ("LF_W03N.ITree.rec", |b| {
                    app(g("LF_W03N.ITree"), b[b.len() - 1].clone())
                }),
                ("LF_W03N.ITree.rec_1", |_| {
                    list0(app(g("LF_W03N.ITree"), numeral(0)))
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_NestedFrontier.DTree", "LF_W03_NestedFrontier"),
            FRONTIER,
            "121f5e30f1059ff2e510d20ee7e24a4b38ff4bf28034eb4478fb265492c5f891",
            69_273,
            &["LF_W03N.DTree"],
            "LF_W03N.dLeaf",
            vec![
                ("LF_W03N.DTree.rec", |_| g("LF_W03N.DTree")),
                ("LF_W03N.DTree.rec_1", |_| list0(g("LF_W03N.DTree"))),
            ],
        ),
        witness(
            fixture!("LF_W03_NestedFrontier.MTree", "LF_W03_NestedFrontier"),
            FRONTIER,
            "89ef5709a003f77fcb1106e6713853eaab0ffee65135f52955f17b727b31f787",
            15_743,
            &["LF_W03N.MTree", "LF_W03N.MForest"],
            "LF_W03N.mEmpty",
            vec![
                ("LF_W03N.MTree.rec", |_| g("LF_W03N.MTree")),
                ("LF_W03N.MForest.rec", |_| g("LF_W03N.MForest")),
                ("LF_W03N.MTree.rec_1", |_| list0(g("LF_W03N.MForest"))),
            ],
        ),
        witness(
            fixture!("LF_W03_F01.VBoxTree", "LF_W03_F01"),
            SHAPES,
            "ab541bd1d0ad47612ec7fb746b1902c24970f0556d8d0d3a09d65fdbc51aa4af",
            26_687,
            &["LF_F01.VBoxTree"],
            "LF_F01.vbLeaf",
            vec![
                ("LF_F01.VBoxTree.rec", |_| g("LF_F01.VBoxTree")),
                ("LF_F01.VBoxTree.rec_1", |_| {
                    apps(g("LF_F01.VBox"), vec![numeral(3), g("LF_F01.VBoxTree")])
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_F01.DPairTree", "LF_W03_F01"),
            SHAPES,
            "f1cbd8e6a6c143de5eb799cf7eb25d7b2f9f2df1154bec81aabf0e269bdd25b7",
            66_749,
            &["LF_F01.DPairTree"],
            "LF_F01.dpLeaf",
            vec![
                ("LF_F01.DPairTree.rec", |_| g("LF_F01.DPairTree")),
                ("LF_F01.DPairTree.rec_1", |_| {
                    app(g("LF_F01.DPair"), g("LF_F01.DPairTree"))
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_F01.SBoxTree", "LF_W03_F01"),
            SHAPES,
            "4a64d51c88616e1c9847d0739ae2bc0c6f81b03280b68c9380d3b5bc13acaa4c",
            11_098,
            &["LF_F01.SBoxTree"],
            "LF_F01.sbLeaf",
            vec![
                ("LF_F01.SBoxTree.rec", |_| g("LF_F01.SBoxTree")),
                ("LF_F01.SBoxTree.rec_1", |_| {
                    app(g("LF_F01.SBox"), g("LF_F01.SBoxTree"))
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_F01.FunTree", "LF_W03_F01"),
            SHAPES,
            "da428efce31b84b2287ffcd2a30a2a45f6eabcc396b9efcf54d14030a3b7b6c8",
            14_392,
            &["LF_F01.FunTree"],
            "LF_F01.ftLeaf",
            vec![
                ("LF_F01.FunTree.rec", |_| g("LF_F01.FunTree")),
                ("LF_F01.FunTree.rec_1", |_| list0(g("LF_F01.FunTree"))),
            ],
        ),
        witness(
            fixture!("LF_W03_F01.Good", "LF_W03_F01"),
            SHAPES,
            "63ea415687b16eb54333b54ec4b8909f8633780e926dfb8fd94e02ef18090bbc",
            9_147,
            &["LF_F01.Good"],
            "LF_F01.good",
            vec![
                ("LF_F01.Good.rec", |_| g("LF_F01.Good")),
                ("LF_F01.Good.rec_1", |_| {
                    apps(g("And"), vec![g("LF_F01.Good"), g("LF_F01.Good")])
                }),
            ],
        ),
        witness(
            fixture!("LF_W03_NestedRose", "LF_W03_NestedRose"),
            ROSE,
            "119c8c198f56a0d7ccbfa6fb46babb12b957da8efd9682e20d757b48a8133049",
            21_988,
            &["LF_W03R.RoseOfTrees"],
            "LF_W03R.rLeaf",
            vec![
                ("LF_W03R.RoseOfTrees.rec", |_| g("LF_W03R.RoseOfTrees")),
                ("LF_W03R.RoseOfTrees.rec_1", |_| {
                    app(g("LF_W03R.Rose"), g("LF_W03R.RoseOfTrees"))
                }),
                ("LF_W03R.RoseOfTrees.rec_2", |_| {
                    list0(app(g("LF_W03R.Rose"), g("LF_W03R.RoseOfTrees")))
                }),
            ],
        ),
    ]
}

fn rows(input: &str) -> Vec<Value> {
    input
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn text(rows: &[Value]) -> String {
    rows.iter().map(|r| format!("{r}\n")).collect()
}
fn spelled(rows: &[Value], id: &Value) -> String {
    let mut parts = Vec::new();
    let mut cur = id.as_u64().unwrap();
    while cur != 0 {
        let row = rows.iter().find(|r| r["in"] == cur).unwrap();
        parts.push(
            row["str"]["str"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| row["num"]["i"].to_string()),
        );
        cur = row["str"]["pre"]
            .as_u64()
            .or_else(|| row["num"]["pre"].as_u64())
            .unwrap();
    }
    parts.reverse();
    parts.join(".")
}
/// The position of the inductive record declaring `family`.
fn block_of(rows: &[Value], family: &str) -> usize {
    rows.iter()
        .position(|r| {
            r["inductive"]["types"]
                .as_array()
                .is_some_and(|types| types.iter().any(|t| spelled(rows, &t["name"]) == family))
        })
        .unwrap()
}
/// Lean's own recursor records for `family`'s block: name ↦ (telescope length, universe count).
fn lean_recursors(rows: &[Value], family: &str) -> Vec<(String, usize, usize)> {
    let block = &rows[block_of(rows, family)];
    block["inductive"]["recs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rec| {
            let count = |k: &str| rec[k].as_u64().unwrap() as usize;
            (
                spelled(rows, &rec["name"]),
                count("numParams")
                    + count("numMotives")
                    + count("numMinors")
                    + count("numIndices")
                    + 1,
                rec["levelParams"].as_array().unwrap().len(),
            )
        })
        .collect()
}
fn pinned(w: &Witness) {
    let p: Value = serde_json::from_str(w.provenance).unwrap();
    assert_eq!(p["source_sha256"], w.source_sha256);
    assert_eq!(p["export_sha256"], w.export_sha256);
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
    assert_eq!(p["records"], w.input.lines().count());
    assert_eq!(w.input.len(), w.bytes);
    for member in w.members {
        let short = member.rsplit('.').next().unwrap();
        assert!(
            w.source.contains(&format!("inductive {short}")),
            "the witness source declares {short}"
        );
    }
}
/// A recursor's type and universe parameters, whether it eliminates into every sort or only `Prop`.
fn recursor(ctx: &Context, name: &str) -> (Vec<String>, Term) {
    match ctx.get_universe_poly(name) {
        Some((levels, ty, _)) => (levels.clone(), ty.clone()),
        None => (
            Vec::new(),
            ctx.get_global(name)
                .unwrap_or_else(|| panic!("{name} is declared"))
                .clone(),
        ),
    }
}

#[test]
fn every_nested_shape_reconstructs_leans_block() {
    for w in witnesses() {
        pinned(&w);
        let rows = rows(w.input);
        let (ctx, report) =
            adapter::import_audited(w.input).unwrap_or_else(|e| panic!("{}: {e:?}", w.members[0]));
        for member in w.members {
            assert!(
                ctx.is_inductive(&n(member)),
                "{member} is a family, not an axiom"
            );
            let usage = axiom_dependencies_of(&ctx, &n(member)).unwrap();
            assert!(
                usage.axioms.is_empty() && usage.unresolved.is_empty(),
                "{member}: {usage:?}"
            );
        }
        let lean = lean_recursors(&rows, w.members[0]);
        assert_eq!(
            lean.len(),
            w.recursors.len(),
            "{}: Lean's recursor inventory",
            w.members[0]
        );
        for (name, major) in &w.recursors {
            let (_, binders, universes) = lean
                .iter()
                .find(|(l, ..)| l == name)
                .unwrap_or_else(|| panic!("Lean exports {name}"));
            let (levels, ty) = recursor(&ctx, &n(name));
            assert_eq!(
                levels.len(),
                *universes,
                "{name}: Lean's universe parameters"
            );
            let mut vars = Vec::new();
            let mut local = ctx.clone();
            let mut rest = ty;
            while let Term::Pi {
                param,
                param_type,
                body_type,
            } = rest
            {
                let bound = format!("b{}", vars.len());
                if vars.len() + 1 == *binders {
                    let expected = major(&vars);
                    assert!(
                        defeq_for_test(&local, &param_type, &expected),
                        "{name}: major premise {param_type}, Lean's {expected}"
                    );
                }
                local = local.extend(&bound, *param_type);
                vars.push(v(&bound));
                rest = logicaffeine_kernel::substitute(&body_type, &param, &v(&bound));
            }
            assert_eq!(vars.len(), *binders, "{name}: Lean's telescope length");
        }
        let user = n(w.user);
        let audit = report.declarations.iter().find(|d| d.name == user).unwrap();
        assert!(
            audit.axioms.is_empty(),
            "Lean: {} depends on no axioms; got {:?}",
            w.user,
            audit.axioms
        );
        for ty in [
            infer_type(&ctx, &Term::Global(user.clone())).unwrap(),
            recheck(&ctx, &Term::Global(user.clone())).unwrap(),
        ] {
            let mut head = &ty;
            while let Term::App(f, _) = head {
                head = f;
            }
            assert!(
                matches!(head, Term::Global(h) | Term::Const { name: h, .. } if w.members.iter().any(|m| *h == n(m))),
                "{}: {ty}",
                w.user
            );
        }
    }
}

#[test]
fn a_dropped_or_swapped_nested_recursor_is_rejected() {
    for w in witnesses() {
        let original = rows(w.input);
        let at = block_of(&original, w.members[0]);
        let mut dropped = original.clone();
        dropped[at]["inductive"]["recs"]
            .as_array_mut()
            .unwrap()
            .pop();
        let mut swapped = original.clone();
        let recs = swapped[at]["inductive"]["recs"].as_array_mut().unwrap();
        let (first, last) = (
            recs[0]["type"].clone(),
            recs[recs.len() - 1]["type"].clone(),
        );
        recs[0]["type"] = last;
        let end = recs.len() - 1;
        recs[end]["type"] = first;
        for (what, forged) in [("dropped", dropped), ("swapped", swapped)] {
            let verdict = adapter::check_str(&text(&forged)).expect_err(what);
            assert!(
                matches!(verdict, Verdict::Reject(_)),
                "{} {what}: {verdict:?}",
                w.members[0]
            );
            assert_eq!(
                verdict.reason().code(),
                DiagnosticCode::InvalidProof,
                "{} {what}",
                w.members[0]
            );
        }
    }
}
