//! `Term::Annotated` is TRANSPARENT: no kernel verdict and no kernel result may depend on it.
//!
//! An annotation records how a subterm was written — a binder's kind (Lean's `BinderInfo`) or
//! where it came from (a source position, a Lean `mdata` entry) — and nothing the kernel decides.
//! The carrier is held to that here, over a corpus of real terms:
//!
//! * every definition of the standard prelude, its type and its body;
//! * the Lean conformance cases' own exports (`lean_conformance/cases_lean414`), imported and taken apart;
//! * the kernel term layer's corpus shapes (`lean_conformance_t00_terms`): shadowed telescopes, spines,
//!   ζ-lets, literal bridges, matches, fixpoints, and terms the kernel REFUSES.
//!
//! Each term is compared with its FULLY ANNOTATED form — a position on every node, a kind on
//! every binder — through every kernel entry point: `infer_type`, `is_subtype`, `normalize`,
//! `recheck`, `double_check`, `axiom_dependencies` and definitional equality. A verdict must be
//! the same; a result must be IDENTICAL (`Term::identical`, which sees annotations), because a
//! kernel result is kernel meaning and carries none. Registering the annotated form must decide
//! as the plain one does and store it erased. Erasure is idempotent, the term store interns both
//! forms to one id, a certificate carries no annotation, the interface printer spells one and the
//! parser reads it back, and a binder kind on an ANONYMOUS `fun` drives implicit insertion.

use logicaffeine_kernel::certificate::{Certificate, TheoremCertificate};
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::interface::printer::print_term;
use logicaffeine_kernel::interface::surface::{resolve_names, Namespaces};
use logicaffeine_kernel::interface::{Repl, TermParser};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::term_store::TermStore;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, double_check, infer_type, instantiate,
    instantiate_universes, is_subtype, normalize_checked, recheck, substitute, surface_elaborate,
    Annotation, BigInt, Context, Literal, MetaCtx, ParamKind, Provenance, SourcePosition,
    StructInfo, Term, Universe,
};
use std::collections::BTreeSet;

// ============================================================================================
// Builders
// ============================================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn let_in(n: &str, ty: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: n.to_string(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    }
}
fn nat() -> Term {
    g("Nat")
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn peano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
fn at(line: u32, t: Term) -> Term {
    Term::annotated(
        Annotation::Source(Provenance::Position(SourcePosition::new(
            "transparency.lean",
            line,
            0,
        ))),
        t,
    )
}
fn kind(k: ParamKind, t: Term) -> Term {
    Term::annotated(Annotation::Binder(k), t)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The FULLY ANNOTATED form of `t`: every node under a source position (numbered in pre-order,
/// so no two annotations are alike), every binder additionally under a binder kind, and every
/// application's head under a Lean `mdata` entry — the annotation shapes an importer and a
/// parser produce, all at once, everywhere.
fn annotate_everywhere(t: &Term) -> Term {
    fn go(t: &Term, serial: &mut u32) -> Term {
        *serial += 1;
        let line = *serial;
        let kinds = [
            ParamKind::Explicit,
            ParamKind::Implicit,
            ParamKind::StrictImplicit,
            ParamKind::Instance,
        ];
        let rebuilt = match t {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => t.clone(),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => kind(
                kinds[line as usize % 4],
                pi(param, go(param_type, serial), go(body_type, serial)),
            ),
            Term::Lambda {
                param,
                param_type,
                body,
            } => kind(
                kinds[line as usize % 4],
                lam(param, go(param_type, serial), go(body, serial)),
            ),
            Term::App(f, a) => {
                let head = Term::annotated(
                    Annotation::Source(Provenance::LeanMetadata {
                        key: "noImplicitLambda".into(),
                        value: "Lean.DataValue.ofBool true".into(),
                    }),
                    go(f, serial),
                );
                app(head, go(a, serial))
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(go(discriminant, serial)),
                motive: Box::new(go(motive, serial)),
                cases: cases.iter().map(|c| go(c, serial)).collect(),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: Box::new(go(body, serial)),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, b)| (n.clone(), go(b, serial)))
                    .collect(),
                index: *index,
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => let_in(name, go(ty, serial), go(value, serial), go(body, serial)),
            Term::Annotated { annotation, term } => {
                Term::annotated(annotation.clone(), go(term, serial))
            }
        };
        at(line, rebuilt)
    }
    go(t, &mut 0)
}

fn annotation_count(t: &Term) -> usize {
    let mut work = vec![t];
    let mut n = 0;
    while let Some(t) = work.pop() {
        match t {
            Term::Annotated { term, .. } => {
                n += 1;
                work.push(term);
            }
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
            Term::Pi {
                param_type,
                body_type: body,
                ..
            }
            | Term::Lambda {
                param_type, body, ..
            } => {
                work.push(param_type);
                work.push(body);
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
        }
    }
    n
}

/// The deepest nesting of `t` — the corpus stays well under the kernel's door either way, so the
/// comparison is about annotations and never about which of two depths a door refused.
fn depth(t: &Term) -> usize {
    let mut best = 0;
    let mut work = vec![(t, 1usize)];
    while let Some((t, d)) = work.pop() {
        best = best.max(d);
        let children: Vec<&Term> = match t {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => vec![],
            Term::Pi {
                param_type,
                body_type: body,
                ..
            }
            | Term::Lambda {
                param_type, body, ..
            } => {
                vec![param_type, body]
            }
            Term::App(f, a) => vec![f, a],
            Term::Match {
                discriminant,
                motive,
                cases,
            } => [discriminant.as_ref(), motive.as_ref()]
                .into_iter()
                .chain(cases.iter())
                .collect(),
            Term::Fix { body, .. } => vec![body],
            Term::MutualFix { defs, .. } => defs.iter().map(|(_, b)| b).collect(),
            Term::Let {
                ty, value, body, ..
            } => vec![ty, value, body],
            Term::Annotated { term, .. } => vec![term],
        };
        work.extend(children.into_iter().map(|c| (c, d + 1)));
    }
    best
}

/// Run `f` on a stack large enough for the fully annotated corpus: the kernel sizes its own, but
/// the test's own clones and comparisons of a doubled-depth term recurse natively.
fn on_a_large_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn_scoped(s, f)
            .expect("spawn the corpus thread")
            .join()
            .expect("the corpus thread completes")
    })
}

// ============================================================================================
// The corpus
// ============================================================================================

/// The kernel term layer's own shapes: the `lean_conformance_t00_terms` cases, and terms it refuses.
fn term_layer_corpus() -> Vec<(&'static str, Term)> {
    let pick_middle = lam("a", nat(), lam("b", nat(), lam("c", nat(), var("b"))));
    let shadowed = lam("x", nat(), lam("x", nat(), lam("x", nat(), var("x"))));
    let apply_twice = lam(
        "h",
        pi("_", nat(), nat()),
        lam("n", nat(), app(var("h"), app(var("h"), var("n")))),
    );
    let eq = |ty: Term, x: Term, y: Term| app(app(app(g("Eq"), ty), x), y);
    let refl = |ty: Term, x: Term| app(app(g("refl"), ty), x);
    let zeta_visible = let_in(
        "n",
        nat(),
        peano(3),
        app(
            lam("h", eq(nat(), var("n"), peano(3)), var("n")),
            refl(nat(), var("n")),
        ),
    );
    let add = g("add");
    vec![
        ("sort", type0()),
        ("prop", Term::Sort(Universe::Prop)),
        ("telescope", pick_middle.clone()),
        (
            "telescope applied",
            app(app(app(pick_middle, peano(1)), peano(2)), peano(3)),
        ),
        (
            "shadowed telescope",
            app(app(app(shadowed.clone(), peano(1)), peano(2)), peano(3)),
        ),
        ("shadowed", shadowed),
        (
            "dependent pi",
            pi("A", type0(), pi("a", var("A"), var("A"))),
        ),
        (
            "polymorphic identity",
            lam("A", type0(), lam("a", var("A"), var("a"))),
        ),
        (
            "higher order",
            app(app(apply_twice.clone(), g("Succ")), peano(3)),
        ),
        (
            "right nested refused",
            app(apply_twice, app(g("Succ"), peano(3))),
        ),
        (
            "zeta let",
            let_in("n", nat(), peano(3), app(g("Succ"), var("n"))),
        ),
        ("zeta visible", zeta_visible),
        (
            "nested lets",
            let_in(
                "a",
                nat(),
                peano(2),
                let_in(
                    "b",
                    nat(),
                    app(g("Succ"), var("a")),
                    app(g("Succ"), var("b")),
                ),
            ),
        ),
        (
            "type level let",
            let_in("A", type0(), nat(), pi("_", var("A"), var("A"))),
        ),
        ("nat literal", Term::Lit(Literal::Nat(BigInt::from_i64(3)))),
        ("int literal", Term::Lit(Literal::Int(-3))),
        ("text literal", Term::Lit(Literal::Text("hello".into()))),
        (
            "literal bridge",
            eq(
                nat(),
                Term::Lit(Literal::Nat(BigInt::from_i64(3))),
                peano(3),
            ),
        ),
        ("proof by refl", refl(nat(), peano(2))),
        ("addition", app(app(add.clone(), peano(2)), peano(2))),
        (
            "addition of literals",
            app(
                app(add, Term::Lit(Literal::Int(40))),
                Term::Lit(Literal::Int(2)),
            ),
        ),
        (
            "match",
            Term::Match {
                discriminant: Box::new(peano(2)),
                motive: Box::new(lam("_", nat(), nat())),
                cases: vec![peano(7), lam("k", nat(), var("k"))],
            },
        ),
        (
            "structural fixpoint",
            Term::Fix {
                name: "f".into(),
                body: Box::new(lam(
                    "n",
                    nat(),
                    Term::Match {
                        discriminant: Box::new(var("n")),
                        motive: Box::new(lam("_", nat(), nat())),
                        cases: vec![g("Zero"), lam("k", nat(), app(var("f"), var("k")))],
                    },
                )),
            },
        ),
        ("not a function", app(g("Zero"), g("Zero"))),
        ("hole domain", lam("a", Term::Hole, var("a"))),
        ("unbound", app(g("Succ"), var("free"))),
        ("unbound global", app(g("id"), peano(3))),
        ("type mismatch", app(g("Succ"), g("true"))),
    ]
}

/// Every definition of a context, sorted by name, as `(name, type, body)`.
fn definitions(ctx: &Context) -> Vec<(String, Term, Term)> {
    let mut defs: Vec<(String, Term, Term)> = ctx
        .iter_definitions()
        .map(|(n, ty, body)| (n.to_string(), ty.clone(), body.clone()))
        .collect();
    defs.sort_by(|a, b| a.0.cmp(&b.0));
    defs
}

/// The Lean conformance cases' own exports, as Lean 4.14 wrote them.
const LEAN_CASES: &[(&str, &str)] = &[
    (
        "T00-002",
        include_str!("fixtures/lean_conformance/cases_lean414/T00-002.ndjson"),
    ),
    (
        "T00-004",
        include_str!("fixtures/lean_conformance/cases_lean414/T00-004.ndjson"),
    ),
    (
        "T00-005",
        include_str!("fixtures/lean_conformance/cases_lean414/T00-005.ndjson"),
    ),
    (
        "T00-014",
        include_str!("fixtures/lean_conformance/cases_lean414/T00-014.ndjson"),
    ),
];

// ============================================================================================
// The comparison
// ============================================================================================

/// A kernel answer, reduced to what must be the same: a result's EXACT term (annotations
/// included) or the error it failed with.
fn same_result<E: std::fmt::Debug>(
    what: &str,
    plain: &Result<Term, E>,
    annotated: &Result<Term, E>,
    failures: &mut Vec<String>,
) {
    match (plain, annotated) {
        (Ok(p), Ok(a)) if p.identical(a) => {}
        (Err(p), Err(a)) if format!("{p:?}") == format!("{a:?}") => {}
        _ => failures.push(format!(
            "{what}: plain {plain:?}\n  annotated {annotated:?}"
        )),
    }
}

/// Every kernel entry point, on `t` and on its fully annotated form, in `ctx`. `expected` is a
/// type the term claims, compared through `is_subtype` both ways.
fn compare(
    ctx: &Context,
    label: &str,
    t: &Term,
    expected: Option<&Term>,
    failures: &mut Vec<String>,
) {
    let annotated = annotate_everywhere(t);
    assert!(
        annotation_count(&annotated) > annotation_count(t),
        "{label}: the fully annotated form carries annotations"
    );
    assert!(
        annotated.strip_annotations().identical(t),
        "{label}: erasure recovers the term exactly"
    );
    assert!(
        annotated
            .strip_annotations()
            .strip_annotations()
            .identical(t),
        "{label}: erasure is idempotent"
    );
    assert_eq!(
        annotated, *t,
        "{label}: `==` is kernel meaning, and an annotation is none of it"
    );

    let mut store = TermStore::new();
    let (plain_id, annotated_id) = (store.intern(t), store.intern(&annotated));
    assert_eq!(
        plain_id, annotated_id,
        "{label}: an annotation never splits sharing"
    );
    assert!(
        store.extract(annotated_id).identical(t),
        "{label}: the store holds the erased term"
    );

    let inferred = infer_type(ctx, t);
    same_result(
        &format!("{label}: infer_type"),
        &inferred,
        &infer_type(ctx, &annotated),
        failures,
    );
    same_result(
        &format!("{label}: recheck"),
        &recheck(ctx, t),
        &recheck(ctx, &annotated),
        failures,
    );
    same_result(
        &format!("{label}: normalize"),
        &normalize_checked(ctx, t),
        &normalize_checked(ctx, &annotated),
        failures,
    );
    let (plain_check, annotated_check) = (double_check(ctx, t), double_check(ctx, &annotated));
    if plain_check != annotated_check {
        failures.push(format!(
            "{label}: double_check {plain_check:?} / {annotated_check:?}"
        ));
    }
    let (plain_axioms, annotated_axioms) = (
        axiom_dependencies(ctx, t),
        axiom_dependencies(ctx, &annotated),
    );
    if plain_axioms != annotated_axioms {
        failures.push(format!(
            "{label}: axiom_dependencies {plain_axioms:?} / {annotated_axioms:?}"
        ));
    }
    if inferred.is_ok() {
        for (a, b, name) in [
            (t, &annotated, "t ≡ annotated"),
            (&annotated, t, "annotated ≡ t"),
        ] {
            if !defeq_for_test(ctx, a, b) {
                failures.push(format!("{label}: def_eq {name} refused"));
            }
            if !is_subtype(ctx, a, b) {
                failures.push(format!("{label}: is_subtype {name} refused"));
            }
        }
    }
    if let (Ok(ty), Some(expected)) = (&inferred, expected) {
        let annotated_ty = annotate_everywhere(ty);
        let annotated_expected = annotate_everywhere(expected);
        let plain = (
            is_subtype(ctx, ty, expected),
            defeq_for_test(ctx, ty, expected),
        );
        let annotated = (
            is_subtype(ctx, &annotated_ty, &annotated_expected),
            defeq_for_test(ctx, &annotated_ty, &annotated_expected),
        );
        if plain != annotated {
            failures.push(format!(
                "{label}: against its type {plain:?} / {annotated:?}"
            ));
        }
    }
}

/// Registering the annotated declaration decides as the plain one does, and stores it ERASED.
fn compare_registration(
    ctx: &Context,
    label: &str,
    ty: &Term,
    body: &Term,
    failures: &mut Vec<String>,
) {
    let fresh = format!("annotation_transparency_{label}");
    let (mut plain, mut annotated) = (ctx.clone(), ctx.clone());
    let plain_verdict = plain.declare(&fresh, ty.clone(), body.clone());
    let annotated_verdict =
        annotated.declare(&fresh, annotate_everywhere(ty), annotate_everywhere(body));
    if format!("{plain_verdict:?}") != format!("{annotated_verdict:?}") {
        failures.push(format!(
            "{label}: declare {plain_verdict:?} / {annotated_verdict:?}"
        ));
    }
    match (
        plain.get_definition_body(&fresh),
        annotated.get_definition_body(&fresh),
    ) {
        (Some(p), Some(a)) => {
            if !p.identical(a) || a.has_annotations() {
                failures.push(format!("{label}: the stored body is not the erased body"));
            }
            let (pt, at) = (
                plain.get_global(&fresh).unwrap(),
                annotated.get_global(&fresh).unwrap(),
            );
            if !pt.identical(at) || at.has_annotations() {
                failures.push(format!("{label}: the stored type is not the erased type"));
            }
        }
        (None, None) => {}
        (p, a) => failures.push(format!("{label}: stored {p:?} / {a:?}")),
    }
}

fn report(failures: Vec<String>, checked: usize) {
    assert!(
        failures.is_empty(),
        "{} of {checked} comparisons changed under an annotation:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ============================================================================================
// The locks
// ============================================================================================

#[test]
fn every_prelude_definition_checks_reduces_and_registers_the_same_fully_annotated() {
    on_a_large_stack(|| {
        let ctx = std_ctx();
        let defs = definitions(&ctx);
        assert!(
            defs.len() > 50,
            "the prelude corpus is the prelude: {} definitions",
            defs.len()
        );
        let mut failures = Vec::new();
        for (name, ty, body) in &defs {
            if depth(body).max(depth(ty)) > 400 {
                continue;
            }
            compare(&ctx, &format!("{name} (type)"), ty, None, &mut failures);
            compare(&ctx, name, body, Some(ty), &mut failures);
            compare_registration(&ctx, name, ty, body, &mut failures);
        }
        report(failures, defs.len());
    });
}

#[test]
fn the_term_layer_corpus_gives_the_same_verdicts_accepted_and_refused() {
    on_a_large_stack(|| {
        let ctx = std_ctx();
        let corpus = term_layer_corpus();
        let refused = corpus
            .iter()
            .filter(|(_, t)| infer_type(&ctx, t).is_err())
            .count();
        assert!(
            refused >= 5,
            "the corpus carries the terms the kernel refuses, not only accepted ones"
        );
        let mut failures = Vec::new();
        for (label, t) in &corpus {
            compare(&ctx, label, t, None, &mut failures);
            if let Ok(ty) = infer_type(&ctx, t) {
                compare_registration(&ctx, &label.replace(' ', "_"), &ty, t, &mut failures);
            }
        }
        report(failures, corpus.len());
    });
}

#[test]
fn the_lean_conformance_cases_imported_check_the_same_fully_annotated() {
    on_a_large_stack(|| {
        let mut checked = 0;
        let mut failures = Vec::new();
        for (case, export) in LEAN_CASES {
            let (imported, _) =
                adapter::import(export).unwrap_or_else(|e| panic!("{case} imports: {e:?}"));
            for (name, ty, body) in definitions(&*imported) {
                if !name.contains(&case.replace('-', "_")) || depth(&body).max(depth(&ty)) > 400 {
                    continue;
                }
                checked += 1;
                compare(
                    &imported,
                    &format!("{case} {name}"),
                    &body,
                    Some(&ty),
                    &mut failures,
                );
            }
        }
        assert!(
            checked >= 8,
            "the Lean cases contribute their own declarations: {checked}"
        );
        report(failures, checked);
    });
}

#[test]
fn the_context_stores_every_term_erased() {
    let ctx = std_ctx();
    let annotated_nat = at(1, kind(ParamKind::Implicit, pi("_", nat(), at(2, nat()))));
    let local = ctx.extend("x", annotated_nat.clone());
    assert!(
        !local.get("x").unwrap().has_annotations(),
        "a local binding is stored erased"
    );
    assert!(local.get("x").unwrap().identical(&pi("_", nat(), nat())));

    let mut ctx = ctx;
    ctx.add_declaration("annotated_axiom", annotated_nat.clone());
    assert!(
        !ctx.get_global("annotated_axiom").unwrap().has_annotations(),
        "a declaration"
    );
    ctx.add_instance(annotated_nat.clone(), at(3, g("Zero")));
    assert!(ctx
        .instances()
        .iter()
        .all(|(t, v)| !t.has_annotations() && !v.has_annotations()));
    ctx.add_coercion(at(4, g("Bool")), at(5, nat()), at(6, g("boolToNat")));
    assert!(ctx
        .coercions()
        .iter()
        .all(|(a, b, c)| ![a, b, c].iter().any(|t| t.has_annotations())));
    ctx.set_param_defaults("annotated_axiom", vec![("m".into(), Some(at(7, peano(3))))]);
    assert!(ctx
        .param_defaults("annotated_axiom")
        .unwrap()
        .iter()
        .all(|(_, d)| !d.as_ref().unwrap().has_annotations()));
    ctx.add_inductive("AnnotatedBox", at(8, type0()));
    ctx.add_constructor(
        "AnnotatedBox_mk",
        "AnnotatedBox",
        at(9, pi("_", at(10, nat()), g("AnnotatedBox"))),
    );
    assert!(
        ctx.get_constructors("AnnotatedBox")
            .iter()
            .all(|(_, ty)| !ty.has_annotations()),
        "a constructor is stored erased"
    );
    ctx.add_structure(
        "AnnotatedPair",
        &[("A", at(11, type0()))],
        &[("first", at(12, var("A")))],
    );
    let info = ctx
        .struct_info("AnnotatedPair")
        .expect("the structure registers");
    assert!(
        info.params
            .iter()
            .chain(info.fields.iter())
            .all(|(_, ty)| !ty.has_annotations()),
        "a structure's parameter and field telescopes are stored erased: {info:?}"
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );

    let mut cell = Repl::new();
    for line in [
        "Inductive AnnotatedCell : Type := annotated_cell_mk : Nat -> AnnotatedCell.",
        "Definition annotated_cell_get : AnnotatedCell -> Nat := fun (c : AnnotatedCell) => \
         match c return (fun (q : AnnotatedCell) => Nat) with fun (n : Nat) => n end.",
    ] {
        cell.execute(line)
            .unwrap_or_else(|e| panic!("{line}: {e:?}"));
    }
    let mut cell = cell.context().clone();
    assert!(
        cell.struct_info("AnnotatedCell").is_none(),
        "no structure η before the metadata door grants it"
    );
    cell.register_struct_info(
        "AnnotatedCell",
        StructInfo {
            mk: "annotated_cell_mk".into(),
            num_params: 0,
            projections: vec!["annotated_cell_get".into()],
            params: vec![],
            fields: vec![("value".into(), at(13, kind(ParamKind::Implicit, nat())))],
            parents: vec![],
            out_params: vec![],
        },
    );
    let info = cell
        .struct_info("AnnotatedCell")
        .expect("the metadata door grants a real projection");
    assert!(
        info.params
            .iter()
            .chain(info.fields.iter())
            .all(|(_, ty)| !ty.has_annotations()),
        "structure metadata registered directly is stored erased too: {info:?}"
    );
    assert!(
        info.fields[0].1.identical(&nat()),
        "and erased to exactly its kernel term: {info:?}"
    );
    assert!(
        cell.refused_declarations().is_empty(),
        "{:?}",
        cell.refused_declarations()
    );
}

#[test]
fn a_certificate_carries_kernel_meaning_and_no_annotation() {
    let proof = app(app(g("refl"), nat()), peano(2));
    let claim = app(app(app(g("Eq"), nat()), peano(2)), peano(2));
    let cert = Certificate::new(annotate_everywhere(&proof), annotate_everywhere(&claim));
    assert!(!cert.proof_term.has_annotations() && !cert.claimed_type.has_annotations());
    let json = serde_json::to_string(&cert).expect("an erased certificate serializes");
    assert!(!json.contains("Annotated"), "{json}");
    assert_eq!(
        json,
        serde_json::to_string(&Certificate::new(proof.clone(), claim.clone())).unwrap()
    );
    logicaffeine_kernel::certificate::recheck(&cert).expect("the erased certificate rechecks");

    let theorem = TheoremCertificate::new(
        "t",
        annotate_everywhere(&proof),
        annotate_everywhere(&claim),
        vec![],
        vec![],
    );
    assert!(!serde_json::to_string(&theorem)
        .unwrap()
        .contains("Annotated"));

    // An annotation has NO serialized form: a capture that forgot to erase fails loudly rather
    // than widening the certificate language.
    let raw = serde_json::to_string(&annotate_everywhere(&proof));
    assert!(
        raw.is_err(),
        "an annotated term does not serialize: {raw:?}"
    );
    let forged = r#"{"Annotated":{"annotation":{"Binder":"Implicit"},"term":{"Global":"Zero"}}}"#;
    assert!(
        serde_json::from_str::<Term>(forged).is_err(),
        "nor does one deserialize"
    );
}

#[test]
fn the_printer_spells_every_annotation_and_the_parser_reads_it_back() {
    let annotated = [
        kind(
            ParamKind::Implicit,
            lam("A", type0(), lam("a", var("A"), var("a"))),
        ),
        kind(
            ParamKind::StrictImplicit,
            pi("A", type0(), pi("_", var("A"), var("A"))),
        ),
        kind(ParamKind::Instance, lam("A", type0(), var("A"))),
        kind(ParamKind::Explicit, lam("a", nat(), var("a"))),
        at(1, peano(3)),
        app(at(2, g("Succ")), at(3, g("Zero"))),
        app(g("Succ"), at(4, app(g("Succ"), g("Zero")))),
        pi("_", at(5, nat()), nat()),
        Term::annotated(
            Annotation::Source(Provenance::LeanMetadata {
                key: "pp.analysis \"quoted\"".into(),
                value: "Lean.DataValue.ofBool true\nsecond\tline\\".into(),
            }),
            at(6, kind(ParamKind::Implicit, lam("x", nat(), var("x")))),
        ),
        Term::annotated(
            Annotation::Source(Provenance::Position(SourcePosition::new(
                "dir/Main File.lean",
                12,
                7,
            ))),
            let_in("k", nat(), peano(1), app(g("Succ"), var("k"))),
        ),
        annotate_everywhere(&app(lam("n", nat(), app(g("Succ"), var("n"))), peano(1))),
    ];
    // Every binder the printer is free to rename (an unreferenced one prints as an arrow and reads
    // back as `_`) is already `_` here, so the read-back is IDENTICAL, annotations and names both.
    let known: BTreeSet<String> = ["Nat", "Succ", "Zero"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let known_fn = |k: &str| known.contains(k);
    for t in &annotated {
        let text = print_term(t).unwrap_or_else(|e| panic!("{t:?} prints: {e}"));
        assert!(text.contains("@["), "an annotation is SPELLED: {text}");
        let parsed = TermParser::parse(&text).unwrap_or_else(|e| panic!("{text} parses: {e:?}"));
        let read = resolve_names(&Namespaces::new(), &known_fn, &mut Vec::new(), &parsed)
            .unwrap_or_else(|e| panic!("{text} resolves: {e:?}"))
            .into_owned();
        assert!(
            read.identical(t),
            "`{text}` reads back as\n{read:?}\nnot\n{t:?}"
        );
    }
    for malformed in [
        "@[binder] Zero",
        "@[binder sideways] Zero",
        "@[source \"no-position\"] Zero",
        "@[source \"a.lean:0:0\"] Zero",
        "@[mdata \"key\"] Zero",
        "@[color \"red\"] Zero",
        "@[binder implicit Zero",
    ] {
        assert!(
            TermParser::parse(malformed).is_err(),
            "`{malformed}` is refused"
        );
    }
}

#[test]
fn a_binder_kind_on_an_anonymous_fun_inserts_the_implicit_argument() {
    let ctx = std_ctx();
    let poly_id = lam("A", type0(), lam("a", var("A"), var("a")));
    let implicit_id = kind(ParamKind::Implicit, poly_id.clone());

    // With the kind recorded on the anonymous binder, `id Zero` is `id Nat Zero`.
    let elaborated = surface_elaborate(&ctx, &app(implicit_id.clone(), g("Zero")))
        .expect("the implicit is inserted and solved");
    assert!(!elaborated
        .strip_annotations()
        .identical(&app(poly_id.clone(), g("Zero"))));
    assert_eq!(
        infer_type(&ctx, &elaborated).expect("the elaborated application checks"),
        nat(),
        "the inserted argument is `Nat`, solved from `Zero`"
    );
    match elaborated.strip_annotations() {
        Term::App(f, a) => {
            assert_eq!(*a, g("Zero"));
            assert!(matches!(f.as_ref(), Term::App(_, first) if first.as_ref() == &nat()));
        }
        other => panic!("an application of the identity: {other:?}"),
    }
    let mut head = &elaborated;
    while let Term::App(f, _) = head {
        head = f;
    }
    assert_eq!(
        head.binder_kind(),
        Some(ParamKind::Implicit),
        "the annotation stays on its head"
    );

    // The same anonymous `fun` WITHOUT the kind takes its first argument explicitly: the
    // annotation is what changed the elaboration, and nothing else.
    let explicit = surface_elaborate(&ctx, &app(poly_id.clone(), g("Zero")))
        .expect("the explicit application elaborates as written");
    assert!(explicit.identical(&app(poly_id, g("Zero"))));
    assert!(infer_type(&ctx, &explicit).is_err(), "`Zero` is not a type");

    // A kind on a binder that is not the head's first leaves the explicit prefix alone.
    let second_implicit = lam(
        "n",
        nat(),
        kind(ParamKind::Implicit, lam("A", type0(), var("n"))),
    );
    let elaborated = surface_elaborate(&ctx, &app(second_implicit, peano(1)))
        .expect("an explicit first argument, an implicit second");
    assert!(infer_type(&ctx, &elaborated).is_ok(), "{elaborated:?}");
}

#[test]
fn a_syntactic_operation_reaches_under_an_annotation_and_keeps_it() {
    // Substitution: the annotated occurrence is replaced, and the annotation stays where it was.
    let body = at(1, app(g("Succ"), at(2, var("x"))));
    let substituted = substitute(&body, "x", &g("Zero"));
    assert!(
        substituted.identical(&at(1, app(g("Succ"), at(2, g("Zero"))))),
        "an annotation hides no occurrence from substitution: {substituted:?}"
    );
    // Capture-avoiding under an annotated binder: the binder is renamed, the annotation kept.
    let binder = kind(
        ParamKind::Implicit,
        lam("y", nat(), at(3, app(var("x"), var("y")))),
    );
    let substituted = substitute(&binder, "x", &var("y"));
    assert_eq!(
        substituted.binder_kind(),
        Some(ParamKind::Implicit),
        "the kind stays on its binder"
    );
    match substituted.peel_annotations() {
        Term::Lambda { param, body, .. } => {
            assert_ne!(
                param, "y",
                "the binder is renamed away from the replacement's `y`"
            );
            assert!(
                body.identical(&at(3, app(var("y"), var(param)))),
                "{body:?}"
            );
        }
        other => panic!("still a binder: {other:?}"),
    }
    // Universe instantiation and metavariable instantiation, the elaborator's two others.
    let levelled = at(4, Term::Sort(Universe::Var("u".into())));
    let subst = std::iter::once(("u".to_string(), Universe::Type(0))).collect();
    assert!(instantiate_universes(&levelled, &subst).identical(&at(4, type0())));
    let mut mctx = MetaCtx::new();
    mctx.solve("?m", nat());
    assert!(instantiate(&at(5, var("?m")), &mctx).identical(&at(5, nat())));
}

#[test]
fn the_vernacular_reads_an_annotation_and_the_repl_checks_through_it() {
    let mut repl = Repl::new();
    // A binder kind written on an anonymous `fun` inserts the implicit argument end to end:
    // parse, name resolution, elaboration, the kernel.
    let checked = repl
        .execute("Check (@[binder implicit] (fun (A : Type) => fun (a : A) => a)) Zero.")
        .expect("the implicit is inserted and the application checks");
    assert!(checked.ends_with(": Nat"), "{checked}");
    // Without it the first argument is explicit, and `Zero` is not a type.
    assert!(repl
        .execute("Check (fun (A : Type) => fun (a : A) => a) Zero.")
        .is_err());
    // A source position and a Lean `mdata` entry change nothing the kernel decides.
    let plain = repl
        .execute("Eval Succ (Succ Zero).")
        .expect("plain evaluation");
    let annotated = repl
        .execute("Eval @[source \"repl.lean:1:5\"] (Succ (@[mdata \"k\" \"v\"] (Succ Zero))).")
        .expect("annotated evaluation");
    assert_eq!(
        plain, annotated,
        "an annotation is erased before the kernel reduces"
    );
    // A declaration is stored erased, whatever the source annotated.
    repl.execute("Definition two : Nat := @[source \"repl.lean:3:0\"] (Succ (Succ Zero)).")
        .expect("an annotated body declares");
    assert_eq!(repl.execute("Eval two.").unwrap(), plain);
}

#[test]
fn every_door_erases_before_the_depth_door_measures() {
    // A spine the kernel admits (1001 levels, under its 1600) whose fully annotated form is twice
    // as deep. An annotation adds nesting to the representation and none to the term, so every
    // entry point must decide the annotated spine exactly as the plain one — which it can only do
    // if it erases BEFORE it measures.
    on_a_large_stack(|| {
        let ctx = std_ctx();
        let plain = peano(1000);
        let annotated = annotate_everywhere(&plain);
        assert!(
            depth(&plain) < 1600 && depth(&annotated) > 1600,
            "the fixture straddles the door"
        );
        let mut failures = Vec::new();
        same_result(
            "infer_type",
            &infer_type(&ctx, &plain),
            &infer_type(&ctx, &annotated),
            &mut failures,
        );
        same_result(
            "recheck",
            &recheck(&ctx, &plain),
            &recheck(&ctx, &annotated),
            &mut failures,
        );
        same_result(
            "normalize",
            &normalize_checked(&ctx, &plain),
            &normalize_checked(&ctx, &annotated),
            &mut failures,
        );
        let (p, a) = (double_check(&ctx, &plain), double_check(&ctx, &annotated));
        if p != a {
            failures.push(format!("double_check {p:?} / {a:?}"));
        }
        for (name, verdict) in [
            (
                "is_subtype annotated ≤ plain",
                is_subtype(&ctx, &annotated, &plain),
            ),
            (
                "is_subtype plain ≤ annotated",
                is_subtype(&ctx, &plain, &annotated),
            ),
            (
                "is_subtype annotated ≤ annotated",
                is_subtype(&ctx, &annotated, &annotated),
            ),
            (
                "def_eq annotated ≡ plain",
                defeq_for_test(&ctx, &annotated, &plain),
            ),
            (
                "def_eq annotated ≡ annotated",
                defeq_for_test(&ctx, &annotated, &annotated.clone()),
            ),
        ] {
            if !verdict {
                failures.push(format!("{name} refused"));
            }
        }
        assert!(
            infer_type(&ctx, &plain).is_ok(),
            "the plain spine is admitted"
        );
        report(failures, 8);
        let mut spine = annotated;
        // Dismantle the doubled spine iteratively: the derived `Drop` recurses once per level.
        loop {
            match std::mem::replace(&mut spine, Term::Hole) {
                Term::App(_, a) => spine = *a,
                Term::Annotated { term, .. } => spine = *term,
                _ => break,
            }
        }
    });
}
