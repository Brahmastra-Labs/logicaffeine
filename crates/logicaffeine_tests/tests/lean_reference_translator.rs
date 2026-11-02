//! W05 / R8 — the reference translator is the independent oracle a result's STATEMENT is compared
//! against. Three readings of the same declaration are held against each other here: the corpus's
//! goal written by hand, the production importer's checked type, and the reference translation.
//! Any two agreeing while the third disagrees is a failure of one of them, and this file says which.
mod lean_package_support;

use lean_package_support::{small_theorem, Export};
use logicaffeine_kernel::interface::lean::reference::{
    kernel_name_of_components, reference_declaration, reference_type, ReferenceKind,
};
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{defeq_for_test, BigInt, Context, Literal, Term, Universe};
use std::path::Path;

const T06_002: &str = include_str!("fixtures/lean_conformance/cases_lean414/T06-002.ndjson");

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
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn eq(a: Term, x: Term, y: Term) -> Term {
    apps(
        Term::Const {
            name: r#"/s:"Eq""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![a, x, y],
    )
}
fn refl(a: Term, x: Term) -> Term {
    apps(
        Term::Const {
            name: r#"/s:"Eq"/s:"refl""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![a, x],
    )
}

/// Corpus case T06-002's goals, written by hand from the Lean source — the third reading.
fn hand_goal(short: &str) -> Term {
    match short {
        "transport" | "transportRaw" => pi(
            "A",
            type0(),
            pi(
                "P",
                pi("x", v("A"), Term::Sort(Universe::Prop)),
                pi(
                    "a",
                    v("A"),
                    pi(
                        "b",
                        v("A"),
                        pi(
                            "h",
                            eq(v("A"), v("a"), v("b")),
                            pi("pa", app(v("P"), v("a")), app(v("P"), v("b"))),
                        ),
                    ),
                ),
            ),
        ),
        "symmViaRec" => pi(
            "A",
            type0(),
            pi(
                "a",
                v("A"),
                pi(
                    "b",
                    v("A"),
                    pi("h", eq(v("A"), v("a"), v("b")), eq(v("A"), v("b"), v("a"))),
                ),
            ),
        ),
        "viaMpr" => pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                pi(
                    "h",
                    eq(g("Nat"), v("a"), v("b")),
                    pi(
                        "p",
                        eq(g("Nat"), v("b"), v("b")),
                        eq(g("Nat"), v("a"), v("a")),
                    ),
                ),
            ),
        ),
        "bigMotive" => pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                pi(
                    "h",
                    eq(g("Nat"), v("a"), v("b")),
                    pi(
                        "F",
                        pi("n", g("Nat"), type0()),
                        pi("x", app(v("F"), v("a")), app(v("F"), v("b"))),
                    ),
                ),
            ),
        ),
        "jDependent" => {
            let motive = pi(
                "b",
                v("A"),
                pi("h", eq(v("A"), v("a"), v("b")), Term::Sort(Universe::Prop)),
            );
            let base = apps(v("motive"), vec![v("a"), refl(v("A"), v("a"))]);
            pi(
                "A",
                type0(),
                pi(
                    "a",
                    v("A"),
                    pi(
                        "motive",
                        motive,
                        pi(
                            "base",
                            base,
                            pi(
                                "b",
                                v("A"),
                                pi(
                                    "h",
                                    eq(v("A"), v("a"), v("b")),
                                    apps(v("motive"), vec![v("b"), v("h")]),
                                ),
                            ),
                        ),
                    ),
                ),
            )
        }
        _ => unreachable!(),
    }
}

const ROOTS: [(&str, ReferenceKind); 6] = [
    ("bigMotive", ReferenceKind::Definition),
    ("jDependent", ReferenceKind::Theorem),
    ("symmViaRec", ReferenceKind::Theorem),
    ("transport", ReferenceKind::Theorem),
    ("transportRaw", ReferenceKind::Theorem),
    ("viaMpr", ReferenceKind::Theorem),
];

fn imported_type(ctx: &Context, name: &str) -> Option<Term> {
    ctx.poly_declaration(name)
        .map(|(_, t)| t.clone())
        .or_else(|| ctx.get_global(name).cloned())
}

fn code(verdict: &Verdict) -> DiagnosticCode {
    verdict.reason().code()
}

// -------------------------------------------------------------------------------------------------
// Three readings agree.
// -------------------------------------------------------------------------------------------------

#[test]
fn the_reference_reads_every_t06_002_statement_as_the_hand_written_goal() {
    let (ctx, _) = adapter::import_audited(T06_002).expect("T06-002 imports");
    for (short, kind) in ROOTS {
        let lean = format!("T06_002.{short}");
        let declaration =
            reference_declaration(T06_002, &lean).unwrap_or_else(|e| panic!("{lean}: {e:?}"));
        assert_eq!(declaration.kind, kind, "{lean}");
        assert_eq!(declaration.lean_name, lean);
        assert_eq!(declaration.name, format!("/s:\"T06_002\"/s:{short:?}"));
        assert!(declaration.level_params.is_empty(), "{lean} is monomorphic");
        let hand = hand_goal(short);
        assert!(
            defeq_for_test(&ctx, &declaration.ty, &hand),
            "{lean}: reference ≠ hand-written goal"
        );
        let imported = imported_type(&ctx, &declaration.name).expect("imported");
        assert!(
            defeq_for_test(&ctx, &declaration.ty, &imported),
            "{lean}: reference ≠ imported type"
        );
        // The wrong goal is refused by the same comparison: the oracle is not vacuous.
        let other = if short == "symmViaRec" {
            hand_goal("viaMpr")
        } else {
            hand_goal("symmViaRec")
        };
        assert!(
            !defeq_for_test(&ctx, &declaration.ty, &other),
            "{lean}: the comparison distinguishes goals"
        );
    }
}

#[test]
fn the_fixed_signature_returns_the_same_type_as_the_full_reading() {
    for (short, _) in ROOTS {
        let lean = format!("T06_002.{short}");
        assert_eq!(
            reference_type(T06_002, &lean).unwrap(),
            reference_declaration(T06_002, &lean).unwrap().ty
        );
    }
}

#[test]
fn lean_and_kernel_spellings_name_the_same_declaration() {
    for (short, _) in ROOTS {
        let lean = reference_declaration(T06_002, &format!("T06_002.{short}")).unwrap();
        let kernel =
            reference_declaration(T06_002, &format!("/s:\"T06_002\"/s:{short:?}")).unwrap();
        assert_eq!(lean, kernel, "{short}");
    }
    let nat = reference_declaration(T06_002, "Nat").unwrap();
    assert_eq!(
        (nat.name.as_str(), nat.kind),
        ("Nat", ReferenceKind::Inductive)
    );
    assert_eq!(nat.ty, type0());
    let succ = reference_declaration(T06_002, "Nat.succ").unwrap();
    assert_eq!(
        (succ.name.as_str(), succ.kind),
        ("Succ", ReferenceKind::Constructor)
    );
    assert_eq!(reference_declaration(T06_002, "Succ").unwrap(), succ);
    let rec = reference_declaration(T06_002, "Nat.rec").unwrap();
    assert_eq!(
        (rec.name.as_str(), rec.kind),
        ("Nat/s:\"rec\"", ReferenceKind::Recursor)
    );
}

#[test]
fn universe_parameters_are_read_in_declaration_order_and_agree_with_the_importer() {
    let (ctx, _) = adapter::import_audited(T06_002).unwrap();
    let mut polymorphic = 0;
    for name in [
        "Eq", "Eq.refl", "Eq.rec", "Eq.mpr", "Eq.symm", "cast", "id", "Eq.mp",
    ] {
        let Ok(declaration) = reference_declaration(T06_002, name) else {
            continue;
        };
        let imported = ctx
            .poly_declaration(&declaration.name)
            .map(|(p, _)| p.to_vec())
            .unwrap_or_default();
        assert_eq!(declaration.level_params, imported, "{name}");
        if !imported.is_empty() {
            polymorphic += 1;
        }
    }
    assert!(
        polymorphic >= 3,
        "T06-002 carries polymorphic Eq, Eq.refl and Eq.rec, found {polymorphic}"
    );
    let eq = reference_declaration(T06_002, "Eq").unwrap();
    assert_eq!(eq.level_params, vec!["/s:\"u_1\"".to_string()]);
}

/// Every declaration every accepted fixture imports — definitions, theorems, axioms, inductive
/// types, constructors and recursors — read by the reference translator and compared, by exact
/// conversion, with the importer's checked type. The only permitted refusal is an explicit
/// `UnsupportedFeature` for a projection or string literal inside a statement.
#[test]
fn the_reference_agrees_with_the_importer_on_every_declaration_of_every_accepted_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<_> = ["lean_conformance/cases_lean414", "lean_arena"]
        .iter()
        .flat_map(|dir| std::fs::read_dir(root.join(dir)).unwrap().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ndjson"))
        .collect();
    files.sort();
    let mut compared = 0usize;
    let mut declined = 0usize;
    let mut accepted_fixtures = 0usize;
    for path in files {
        let export = std::fs::read_to_string(&path).unwrap();
        // The large arena inputs belong to the scale lane; the oracle is exercised on every
        // fixture the importer checks within this test's budget.
        if export.len() > 400_000 {
            continue;
        }
        let Ok((ctx, report)) = adapter::import_audited(&export) else {
            continue;
        };
        accepted_fixtures += 1;
        let mut names: Vec<String> = report.declarations.iter().map(|d| d.name.clone()).collect();
        for (inductive, _) in ctx.iter_inductives() {
            names.push(inductive.to_string());
            names.extend(
                ctx.get_constructors(inductive)
                    .into_iter()
                    .map(|(c, _)| c.to_string()),
            );
            let rec = format!("{inductive}/s:\"rec\"");
            if ctx.get_global(&rec).is_some() {
                names.push(rec);
            }
        }
        for name in names {
            if !(name.starts_with('/')
                || name == "Nat"
                || name == "Zero"
                || name == "Succ"
                || name.starts_with("Nat/"))
            {
                continue;
            }
            match reference_declaration(&export, &name) {
                Ok(declaration) => {
                    assert_eq!(declaration.name, name, "{}: identity", path.display());
                    let imported = imported_type(&ctx, &name).expect("imported type");
                    assert!(
                        defeq_for_test(&ctx, &declaration.ty, &imported),
                        "{}: {name}: the reference statement and the imported statement differ",
                        path.display()
                    );
                    let levels = ctx
                        .poly_declaration(&name)
                        .map(|(p, _)| p.to_vec())
                        .unwrap_or_default();
                    assert_eq!(
                        declaration.level_params,
                        levels,
                        "{}: {name}: level telescope",
                        path.display()
                    );
                    compared += 1;
                }
                Err(verdict) => {
                    assert_eq!(
                        code(&verdict),
                        DiagnosticCode::UnsupportedFeature,
                        "{}: {name}: {verdict:?}",
                        path.display()
                    );
                    assert!(
                        verdict.reason().message().contains("projection")
                            || verdict.reason().message().contains("string")
                    );
                    declined += 1;
                }
            }
        }
    }
    assert!(
        accepted_fixtures >= 20,
        "the differential ran over {accepted_fixtures} accepted fixtures"
    );
    assert!(
        compared >= 150,
        "the differential compared {compared} declarations ({declined} declined)"
    );
    assert!(
        declined * 10 <= compared,
        "declines stay the exception: {declined} of {}",
        compared + declined
    );
}

// -------------------------------------------------------------------------------------------------
// The identity contract.
// -------------------------------------------------------------------------------------------------

#[test]
fn primitive_identities_follow_the_kernel_contract() {
    assert_eq!(kernel_name_of_components(&["Nat"]), "Nat");
    assert_eq!(kernel_name_of_components(&["Nat", "zero"]), "Zero");
    assert_eq!(kernel_name_of_components(&["Nat", "succ"]), "Succ");
    assert_eq!(kernel_name_of_components(&["Nat", "add"]), "Nat/s:\"add\"");
    assert_eq!(kernel_name_of_components(&["Quot"]), "kernel.quot.type");
    assert_eq!(kernel_name_of_components(&["Quot", "mk"]), "kernel.quot.mk");
    assert_eq!(
        kernel_name_of_components(&["Quot", "lift"]),
        "kernel.quot.lift"
    );
    assert_eq!(
        kernel_name_of_components(&["Quot", "ind"]),
        "kernel.quot.ind"
    );
    assert_eq!(
        kernel_name_of_components(&["Quot", "sound"]),
        "/s:\"Quot\"/s:\"sound\""
    );
    assert_eq!(
        kernel_name_of_components(&["T06_002", "transport"]),
        "/s:\"T06_002\"/s:\"transport\""
    );
    assert_eq!(
        kernel_name_of_components(&["α", "has \"quotes\""]),
        "/s:\"α\"/s:\"has \\\"quotes\\\"\""
    );
    assert_eq!(
        kernel_name_of_components(&["Natural"]),
        "/s:\"Natural\"",
        "a prefix of Nat is not Nat"
    );
}

#[test]
fn numeric_name_components_are_spelled_as_numbers_and_agree_with_the_importer() {
    let mut e = Export::new();
    let f = e.name("f", 0);
    let f3 = e.num_name(3, f);
    let prop = e.sort(0);
    e.axiom(f3, prop, &[]);
    let text = e.text();
    let declaration = reference_declaration(&text, "f.3").unwrap();
    assert_eq!(declaration.name, "/s:\"f\"/n:3");
    assert_eq!(declaration.kind, ReferenceKind::Axiom);
    assert_eq!(declaration.ty, Term::Sort(Universe::Prop));
    let (ctx, _) = adapter::import_audited(&text).unwrap();
    assert!(
        ctx.get_global("/s:\"f\"/n:3").is_some(),
        "the importer spells the same identity"
    );
}

// -------------------------------------------------------------------------------------------------
// The oracle distinguishes statements.
// -------------------------------------------------------------------------------------------------

#[test]
fn a_changed_goal_in_the_export_changes_the_reference_statement() {
    let honest = small_theorem();
    // The same environment whose theorem claims `Q` instead of `P`.
    let mut e = Export::new();
    let p = e.name("P", 0);
    let proof = e.name("p", 0);
    let t = e.name("T", 0);
    let q = e.name("Q", 0);
    let prop = e.sort(0);
    e.axiom(p, prop, &[]);
    let pc = e.constant(p, &[]);
    e.axiom(proof, pc, &[]);
    e.axiom(q, prop, &[]);
    let qc = e.constant(q, &[]);
    let pv = e.constant(proof, &[]);
    e.thm(t, qc, pv);
    let claimed = reference_type(&e.text(), "T").unwrap();
    let original = reference_type(&honest.text(), "T").unwrap();
    assert_eq!(original, g("/s:\"P\""));
    assert_eq!(claimed, g("/s:\"Q\""));
    let (ctx, _) = adapter::import_audited(&honest.text()).unwrap();
    assert!(!defeq_for_test(&ctx, &claimed, &original));
}

#[test]
fn a_let_bound_statement_keeps_its_let_and_converts_to_its_body() {
    let mut e = Export::new();
    let p = e.name("P", 0);
    let x = e.name("x", 0);
    let t = e.name("T", 0);
    let prop = e.sort(0);
    e.axiom(p, prop, &[]);
    let pc = e.constant(p, &[]);
    let body = e.bvar(0);
    let ty = e.let_in(x, prop, pc, body);
    e.axiom(t, ty, &[]);
    let declaration = reference_declaration(&e.text(), "T").unwrap();
    assert!(
        matches!(declaration.ty, Term::Let { .. }),
        "{:?}",
        declaration.ty
    );
    let (ctx, _) = adapter::import_audited(&e.text()).unwrap();
    assert!(defeq_for_test(&ctx, &declaration.ty, &g("/s:\"P\"")));
}

#[test]
fn metadata_is_transparent_and_binders_resolve_by_depth() {
    let mut e = Export::new();
    let a = e.name("A", 0);
    let b = e.name("b", 0);
    let t = e.name("T", 0);
    let one = e.succ(0);
    let ty0 = e.sort(one);
    // Under one binder `A` is bvar 0; under two it is bvar 1.
    let domain = e.bvar(0);
    let codomain = e.bvar(1);
    let wrapped = e.expr(&format!(
        r#""mdata":{{"expr":{codomain},"data":{{"k":"v"}}}}"#
    ));
    let inner_pi = e.forall(b, domain, wrapped);
    let whole = e.forall(a, ty0, inner_pi);
    e.axiom(t, whole, &[]);
    let declaration = reference_declaration(&e.text(), "T").unwrap();
    // Π(A : Type 0). Π(b : A). A — the inner occurrence is the OUTER binder, not `b`.
    let Term::Pi {
        param: outer,
        body_type,
        ..
    } = &declaration.ty
    else {
        panic!("{:?}", declaration.ty)
    };
    let Term::Pi {
        param: inner_name,
        param_type,
        body_type: codomain,
    } = body_type.as_ref()
    else {
        panic!()
    };
    assert_eq!(param_type.as_ref(), &v(outer));
    assert_eq!(codomain.as_ref(), &v(outer));
    assert_ne!(inner_name, outer);
}

#[test]
fn a_universe_polymorphic_statement_keeps_its_parameter_names() {
    let mut e = Export::new();
    let u = e.name("u", 0);
    let t = e.name("T", 0);
    let lu = e.param(u);
    let su = e.succ(lu);
    let sort = e.sort(su);
    e.axiom(t, sort, &[u]);
    let declaration = reference_declaration(&e.text(), "T").unwrap();
    assert_eq!(declaration.level_params, vec!["/s:\"u\"".to_string()]);
    assert_eq!(
        declaration.ty,
        Term::Sort(Universe::Succ(Box::new(Universe::Var("/s:\"u\"".into()))))
    );
    let (ctx, _) = adapter::import_audited(&e.text()).unwrap();
    assert!(defeq_for_test(
        &ctx,
        &declaration.ty,
        &imported_type(&ctx, "/s:\"T\"").unwrap()
    ));
}

#[test]
fn natural_literals_are_read_exactly_including_ones_beyond_a_machine_word() {
    // `axiom F : Nat → Prop`, then statements `F 3402…457` and `F 000`.
    let mut e2 = Export::new();
    let n = e2.name("Nat", 0);
    let c = e2.name("C", 0);
    let x = e2.name("x", 0);
    let natc = e2.constant(n, &[]);
    let prop = e2.sort(0);
    let family = e2.forall(x, natc, prop);
    let fam = e2.name("F", 0);
    e2.axiom(fam, family, &[]);
    let fc = e2.constant(fam, &[]);
    let big = e2.expr(r#""natVal":"340282366920938463463374607431768211457""#);
    let applied = e2.app(fc, big);
    e2.axiom(c, applied, &[]);
    let zero = e2.expr(r#""natVal":"000""#);
    let applied_zero = e2.app(fc, zero);
    let z = e2.name("Z", 0);
    e2.axiom(z, applied_zero, &[]);
    let declaration = reference_declaration(&e2.text(), "C").unwrap();
    assert_eq!(
        declaration.ty,
        app(
            g("/s:\"F\""),
            Term::Lit(Literal::Nat(
                BigInt::parse_decimal("340282366920938463463374607431768211457").unwrap()
            ))
        )
    );
    assert_eq!(
        reference_type(&e2.text(), "Z").unwrap(),
        app(g("/s:\"F\""), Term::Lit(Literal::Nat(BigInt::zero())))
    );
}

// -------------------------------------------------------------------------------------------------
// Refusals are typed, and never an invented statement.
// -------------------------------------------------------------------------------------------------

#[test]
fn a_missing_declaration_is_refused_not_invented() {
    let e = small_theorem();
    let verdict = reference_type(&e.text(), "Nowhere.toBeFound").unwrap_err();
    assert!(matches!(verdict, Verdict::Reject(_)), "{verdict:?}");
    assert!(verdict.reason().message().contains("Nowhere.toBeFound"));
}

#[test]
fn a_duplicated_declaration_is_refused() {
    let mut e = small_theorem();
    let t = 3; // `small_theorem` names P, p and T, in that order.
    let prop = e.sort(0);
    e.axiom(t, prop, &[]);
    let verdict = reference_type(&e.text(), "T").unwrap_err();
    assert_eq!(
        code(&verdict),
        DiagnosticCode::MalformedExport,
        "{verdict:?}"
    );
}

/// An export whose axiom `T` has the expression `body` for its type.
fn odd_axiom(body: &str) -> String {
    // Name 1 is the structure name a projection cites; expression 0 is its operand.
    let mut e = Export::new();
    e.name("S", 0);
    let t = e.name("T", 0);
    e.sort(0);
    let odd = e.expr(body);
    e.axiom(t, odd, &[]);
    e.text()
}

/// The same export with its header naming `release` (or no release at all).
fn under_release(text: &str, release: Option<&str>) -> String {
    let original = r#""lean":{"version":"4.14.0"}"#;
    assert!(
        text.starts_with(&format!("{{\"meta\":{{{original}")),
        "the writer's header names Lean 4.14.0"
    );
    let replacement = match release {
        Some(release) => format!(r#""lean":{{"version":"{release}"}}"#),
        None => r#""lean":{}"#.to_string(),
    };
    text.replacen(original, &replacement, 1)
}

/// `packing [Char.ofNat c, …]`, spelled out by hand.
fn string_literal(packing: &str, text: &str) -> Term {
    let list = |component: &str| Term::Const {
        name: format!("/s:\"List\"/s:\"{component}\""),
        levels: vec![Universe::Prop],
    };
    let character = g("/s:\"Char\"");
    let mut out = app(list("nil"), character.clone());
    for c in text.chars().rev() {
        let code_point = app(
            g("/s:\"Char\"/s:\"ofNat\""),
            Term::Lit(Literal::Nat(BigInt::from_u64(c as u64))),
        );
        out = apps(list("cons"), vec![character.clone(), code_point, out]);
    }
    app(g(packing), out)
}

#[test]
fn projections_are_declined_and_string_literals_are_read_only_under_a_pinned_release() {
    // A projection has no reference reading under any release.
    let verdict = reference_type(
        &odd_axiom(r#""proj":{"typeName":1,"idx":0,"struct":0}"#),
        "T",
    )
    .unwrap_err();
    assert!(matches!(verdict, Verdict::Decline(_)), "{verdict:?}");
    assert_eq!(
        code(&verdict),
        DiagnosticCode::UnsupportedFeature,
        "{verdict:?}"
    );

    // A string literal reads as its release defines it: exactly, code point by code point.
    let literal = odd_axiom(r#""strVal":"hé𝄞""#);
    assert_eq!(
        reference_type(&literal, "T").unwrap(),
        string_literal("/s:\"String\"/s:\"mk\"", "hé𝄞"),
        "Lean 4.14.0: String.mk over the List Char of Char.ofNat code points"
    );
    for release in ["4.29.1", "4.30.0", "4.33.0"] {
        assert_eq!(
            reference_type(&under_release(&literal, Some(release)), "T").unwrap(),
            string_literal("/s:\"String\"/s:\"ofList\"", "hé𝄞"),
            "Lean {release}: String.ofList"
        );
    }
    assert_eq!(
        reference_type(&odd_axiom(r#""strVal":"""#), "T").unwrap(),
        string_literal("/s:\"String\"/s:\"mk\"", ""),
        "the empty literal is the packed empty list"
    );

    // Under a release with no pinned reading, or none named, a literal is declined, never approximated.
    for release in [Some("4.21.0"), Some("4.99.0"), None] {
        let verdict = reference_type(&under_release(&literal, release), "T").unwrap_err();
        assert!(
            matches!(verdict, Verdict::Decline(_)),
            "{release:?}: {verdict:?}"
        );
        assert_eq!(
            code(&verdict),
            DiagnosticCode::UnsupportedExportVersion,
            "{release:?}: {verdict:?}"
        );
    }
}

#[test]
fn an_unknown_export_version_is_declined_with_its_own_code() {
    let e = small_theorem();
    let text = e.text().replacen(
        "\"format\":{\"version\":\"3.1.0\"}",
        "\"format\":{\"version\":\"9.0.0\"}",
        1,
    );
    let verdict = reference_type(&text, "T").unwrap_err();
    assert_eq!(
        code(&verdict),
        DiagnosticCode::UnsupportedExportVersion,
        "{verdict:?}"
    );
}

#[test]
fn malformed_records_are_malformed_exports() {
    let e = small_theorem();
    for broken in [
        e.text() + "{not json\n",
        e.text()
            .replace(r#""pre":0},"in":1"#, r#""pre":99},"in":1"#),
    ] {
        let verdict = reference_type(&broken, "T").unwrap_err();
        assert_eq!(
            code(&verdict),
            DiagnosticCode::MalformedExport,
            "{verdict:?}"
        );
    }
    let mut loose = Export::new();
    let t = loose.name("T", 0);
    let b = loose.bvar(0);
    loose.axiom(t, b, &[]);
    let verdict = reference_type(&loose.text(), "T").unwrap_err();
    assert_eq!(
        code(&verdict),
        DiagnosticCode::MalformedExport,
        "a loose bound variable: {verdict:?}"
    );
    let mut dangling = Export::new();
    let t = dangling.name("T", 0);
    dangling.raw(format!(
        r#"{{"axiom":{{"name":{t},"levelParams":[],"type":77}}}}"#
    ));
    let verdict = reference_type(&dangling.text(), "T").unwrap_err();
    assert_eq!(
        code(&verdict),
        DiagnosticCode::MalformedExport,
        "a dangling expression: {verdict:?}"
    );
}

#[test]
fn an_exponential_dag_is_refused_before_it_is_unfolded() {
    let mut e = Export::new();
    let t = e.name("T", 0);
    let f = e.name("F", 0);
    let fc = e.constant(f, &[]);
    let mut top = fc;
    for _ in 0..64 {
        top = e.app(top, top);
    }
    e.axiom(t, top, &[]);
    let verdict = reference_type(&e.text(), "T").unwrap_err();
    assert_eq!(code(&verdict), DiagnosticCode::ResourceLimit, "{verdict:?}");
}

#[test]
fn a_deep_telescope_translates_without_native_recursion() {
    const DEPTH: usize = 100_000;
    let mut e = Export::new();
    let t = e.name("T", 0);
    let x = e.name("x", 0);
    let prop = e.sort(0);
    let mut body = prop;
    for _ in 0..DEPTH {
        body = e.forall(x, prop, body);
    }
    e.axiom(t, body, &[]);
    let text = e.text();
    // A small stack: a recursive translation of a hundred-thousand-deep telescope cannot fit.
    let term = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || reference_type(&text, "T").map_err(|v| format!("{v:?}")))
        .unwrap()
        .join()
        .expect("translation stays within a one-megabyte stack")
        .unwrap();
    // Measure and drop the deep tree on a stack sized for it.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let mut depth = 0;
            let mut at = &term;
            while let Term::Pi { body_type, .. } = at {
                depth += 1;
                at = body_type;
            }
            assert_eq!(depth, DEPTH);
            drop(term);
        })
        .unwrap()
        .join()
        .unwrap();
}

// -------------------------------------------------------------------------------------------------
// Independence is structural, and locked.
// -------------------------------------------------------------------------------------------------

#[test]
fn the_reference_translator_shares_no_code_with_the_production_reader() {
    let source = include_str!("../../logicaffeine_kernel/src/interface/lean/reference.rs");
    let code: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "dag::",
        "beta",
        "term_store",
        "TermStore",
        "lean_type",
        "Importer",
        "strings::",
        "normalize",
        "infer_type",
        "runtime_",
        "lazy_conversion",
        "native_conversion",
        "super::exact",
        "super::normal",
        "defeq",
        "def_eq",
        "is_subtype",
        "check_str",
        "import_audited",
        "Context",
    ] {
        assert!(
            !code.contains(forbidden),
            "reference.rs must not use `{forbidden}`"
        );
    }
    let imports: Vec<&str> = code
        .lines()
        .filter(|l| l.trim_start().starts_with("use "))
        .collect();
    assert_eq!(
        imports,
        vec![
            "use super::{declined, rejected, DiagnosticCode, Result};",
            "use crate::{Literal, Term, Universe};",
            "use serde_json::Value;",
            "use std::collections::HashMap;",
        ],
        "the reference translator's whole dependency surface"
    );
}
