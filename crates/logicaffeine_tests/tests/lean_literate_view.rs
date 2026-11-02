//! W04 step 6 — the readable view is LOGOS's own literate English, derived from the checked
//! artifact, and it never changes the checked proposition.
//!
//! A declaration is written in a literate form (`A Color is either red or green or blue.`,
//! `## To shift (n: Nat) -> Nat: Yield Succ n.`) only when reading that text back registers the
//! IDENTICAL kernel declaration. Everything else is written in the vernacular, and the omission of
//! its literate form is recorded with the reason. Each test reads the literate text back in a fresh
//! context by itself — not through the emitter — and compares the result with the import.

use logicaffeine_kernel::interface::lean::emit::{
    self, Correspondence, DeclarationKind, EmittedDocument, Form,
};
use logicaffeine_kernel::interface::lean::{self as adapter, ImportedContext};
use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::{Context, Term, Universe};
use std::collections::HashMap;

const LE_EMIT: &str = include_str!("fixtures/lean_emission/LE_Emit.ndjson");

fn key(parts: &[&str]) -> String {
    parts.iter().map(|p| format!("/s:{p:?}")).collect()
}

fn le(short: &str) -> String {
    key(&["LE_Emit", short])
}

fn emit_plain(target: &str) -> (ImportedContext, EmittedDocument) {
    let imported = adapter::import_audited(LE_EMIT).expect("import").0;
    let doc = emit::emit_document(&imported, target).unwrap_or_else(|e| panic!("{target}: {e:?}"));
    (imported, doc)
}

/// The production import: every declaration of the target's closure retained.
fn emit_retaining(target: &str) -> (ImportedContext, EmittedDocument) {
    let intake = adapter::intake::read(LE_EMIT, &adapter::intake::STAGE_A).expect("intake");
    let closure = intake.closure(target).expect("closure");
    let imported = adapter::import_retaining_closure(LE_EMIT, target, &closure)
        .expect("import")
        .0;
    let doc = emit::emit_document(&imported, target).unwrap_or_else(|e| panic!("{target}: {e:?}"));
    (imported, doc)
}

fn entry<'a>(doc: &'a EmittedDocument, importer_key: &str) -> &'a Correspondence {
    doc.correspondence
        .iter()
        .find(|c| c.importer_key == importer_key)
        .unwrap_or_else(|| panic!("{importer_key}"))
}

/// Binder names erased, bound variables as de Bruijn indices; universes kept structurally.
fn db(term: &Term) -> Term {
    fn go(t: &Term, scope: &mut Vec<String>) -> Term {
        let under = |name: &str, x: &Term, scope: &mut Vec<String>| {
            scope.push(name.to_string());
            let out = go(x, scope);
            scope.pop();
            out
        };
        match t {
            Term::Var(v) => match scope.iter().rev().position(|b| b == v) {
                Some(i) => Term::Var(format!("#{i}")),
                None => Term::Var(format!("free:{v}")),
            },
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: String::new(),
                param_type: Box::new(go(param_type, scope)),
                body_type: Box::new(under(param, body_type, scope)),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: String::new(),
                param_type: Box::new(go(param_type, scope)),
                body: Box::new(under(param, body, scope)),
            },
            Term::App(f, a) => Term::App(Box::new(go(f, scope)), Box::new(go(a, scope))),
            other => other.clone(),
        }
    }
    go(term, &mut Vec::new())
}

/// Rename the import's constants through the document's correspondence.
fn renamed(doc: &EmittedDocument, term: &Term) -> Term {
    let map: HashMap<&str, &str> = doc
        .correspondence
        .iter()
        .map(|c| (c.importer_key.as_str(), c.logos.as_str()))
        .collect();
    fn go(t: &Term, map: &HashMap<&str, &str>) -> Term {
        match t {
            Term::Global(n) => Term::Global(
                map.get(n.as_str())
                    .unwrap_or_else(|| panic!("{n}"))
                    .to_string(),
            ),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(go(param_type, map)),
                body_type: Box::new(go(body_type, map)),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(go(param_type, map)),
                body: Box::new(go(body, map)),
            },
            Term::App(f, a) => Term::App(Box::new(go(f, map)), Box::new(go(a, map))),
            other => other.clone(),
        }
    }
    go(term, &map)
}

/// The inductive `logos` as a context holds it: arity, parameter count, constructors in order.
fn inductive(ctx: &Context, logos: &str) -> (Term, usize, Vec<(String, Term)>) {
    (
        ctx.get_global(logos)
            .unwrap_or_else(|| panic!("{logos} is not declared"))
            .clone(),
        ctx.inductive_num_params(logos),
        ctx.get_constructors(logos)
            .into_iter()
            .map(|(c, t)| (c.to_string(), db(t)))
            .collect(),
    )
}

/// Read ONE literate sentence in a fresh context and hold the inductive it declares against the
/// import.
fn assert_literate_inductive(
    imported: &Context,
    doc: &EmittedDocument,
    lean_key: &str,
    sentence: &str,
) {
    let c = entry(doc, lean_key);
    assert_eq!(
        c.form,
        Form::Literate,
        "{} must be written as literate English",
        c.logos
    );
    assert!(
        doc.source.contains(sentence),
        "the document does not contain {sentence:?}:\n{}",
        doc.source
    );
    assert!(
        !doc.omissions
            .iter()
            .any(|o| o.starts_with(&format!("{}:", c.logos))),
        "{} has a literate form, so no omission is recorded for it",
        c.logos
    );
    let mut fresh = Repl::new();
    fresh
        .execute_document(sentence)
        .unwrap_or_else(|e| panic!("{sentence}: {e}"));
    let (arity, params, ctors) = inductive(fresh.context(), &c.logos);
    let import_ctors: Vec<(String, Term)> = imported
        .get_constructors(lean_key)
        .into_iter()
        .map(|(n, t)| (entry(doc, n).logos.clone(), db(&renamed(doc, t))))
        .collect();
    assert_eq!(
        db(&arity),
        db(&renamed(doc, imported.get_global(lean_key).unwrap())),
        "{}: arity",
        c.logos
    );
    assert_eq!(
        params,
        imported.inductive_num_params(lean_key),
        "{}: parameters",
        c.logos
    );
    assert_eq!(
        ctors, import_ctors,
        "{}: constructors, in order, with their types",
        c.logos
    );
}

#[test]
fn an_enumeration_reads_as_literate_english() {
    let (imported, doc) = emit_retaining(&le("favorite"));
    assert_literate_inductive(
        &imported,
        &doc,
        &le("Color"),
        "A LE_Emit.Color is either Color.red or Color.green or Color.blue.",
    );
}

#[test]
fn constructors_with_fields_read_as_literate_english() {
    let (imported, doc) = emit_retaining(&le("area"));
    assert_literate_inductive(
        &imported,
        &doc,
        &le("Shape"),
        "A LE_Emit.Shape is either Shape.dot or a Shape.segment with value: Nat or a Shape.box with field1: Nat and field2: Nat.",
    );
}

#[test]
fn a_parameterized_recursive_type_reads_as_literate_english() {
    let (imported, doc) = emit_plain(&le("Chain"));
    assert_literate_inductive(
        &imported,
        &doc,
        &le("Chain"),
        "A LE_Emit.Chain of (α: Type) is either Chain.done or a Chain.link with field1: α and field2: Chain of α.",
    );
    // A document that is nothing but literate English still passes the full fresh check.
    emit::verify(&doc).expect("verified");
}

fn nat() -> Term {
    Term::Global("Nat".into())
}

#[test]
fn a_first_order_definition_reads_as_a_to_sentence() {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", nat());
    ctx.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(nat()),
            body_type: Box::new(nat()),
        },
    );
    let shift = key(&["M", "shift"]);
    let ty = Term::Pi {
        param: "k".into(),
        param_type: Box::new(nat()),
        body_type: Box::new(nat()),
    };
    let body = Term::Lambda {
        param: "k".into(),
        param_type: Box::new(nat()),
        body: Box::new(Term::App(
            Box::new(Term::Global("Succ".into())),
            Box::new(Term::Var("k".into())),
        )),
    };
    ctx.declare(&shift, ty.clone(), body.clone())
        .expect("M.shift checks");
    let doc = emit::emit_context(&ctx, &|_| None, &shift).unwrap_or_else(|e| panic!("{e:?}"));
    let c = entry(&doc, &shift);
    assert_eq!((c.logos.as_str(), c.form), ("M.shift", Form::Literate));
    let sentence = "## To M.shift (k: Nat) -> Nat:\n    Yield Succ k.";
    assert!(doc.source.contains(sentence), "{}", doc.source);
    let mut fresh = Repl::new();
    fresh
        .execute_document(sentence)
        .expect("the sentence alone reads back");
    let fresh_ctx = fresh.context();
    assert_eq!(
        db(fresh_ctx.get_definition_type("M.shift").unwrap()),
        db(&ty)
    );
    assert_eq!(
        db(fresh_ctx.get_definition_body("M.shift").unwrap()),
        db(&body)
    );
    emit::verify(&doc).expect("verified");
}

#[test]
fn theorems_and_axioms_have_no_literate_form_and_say_why() {
    let (_, doc) = emit_retaining(&le("uses_oracle"));
    let theorem = entry(&doc, &le("uses_oracle"));
    assert_eq!(
        (theorem.kind, theorem.form),
        (DeclarationKind::Theorem, Form::Vernacular)
    );
    assert!(
        doc.omissions
            .iter()
            .any(|o| o.starts_with("LE_Emit.uses_oracle: ") && o.contains("tactic")),
        "{:?}",
        doc.omissions
    );
    for axiom in ["LE_Emit.Oracle", "LE_Emit.oracle_zero"] {
        assert!(
            doc.omissions
                .iter()
                .any(|o| o.starts_with(&format!("{axiom}: ")) && o.contains("assumption")),
            "{axiom}: {:?}",
            doc.omissions
        );
    }
}

#[test]
fn every_declaration_without_a_literate_form_is_recorded_and_nothing_else_is() {
    for (target, retaining) in [
        (le("favorite"), true),
        (le("area"), true),
        (le("double_two"), true),
        (le("pick_self"), true),
        (le("Chain"), false),
        (key(&["collisions"]), true),
    ] {
        let (_, doc) = if retaining {
            emit_retaining(&target)
        } else {
            emit_plain(&target)
        };
        let mut units: Vec<&Correspondence> = Vec::new();
        for c in &doc.correspondence {
            if c.lines.is_some() && !units.iter().any(|u| u.lines == c.lines) {
                units.push(c);
            }
        }
        for unit in &units {
            let recorded = doc
                .omissions
                .iter()
                .filter(|o| o.starts_with(&format!("{}: ", unit.logos)))
                .count();
            match unit.form {
                Form::Literate => assert_eq!(
                    recorded, 0,
                    "{}: literate, so nothing is omitted",
                    unit.logos
                ),
                Form::Vernacular => {
                    assert_eq!(recorded, 1, "{}: exactly one recorded reason", unit.logos)
                }
                Form::Prelude => panic!("a written unit is never a prelude identity"),
            }
        }
        assert_eq!(
            doc.omissions.len(),
            units.iter().filter(|u| u.form == Form::Vernacular).count(),
            "{target}"
        );
    }
}

#[test]
fn the_literate_view_never_changes_a_checked_declaration() {
    for (target, retaining) in [
        (le("favorite"), true),
        (le("area"), true),
        (le("Chain"), false),
    ] {
        let (imported, doc) = if retaining {
            emit_retaining(&target)
        } else {
            emit_plain(&target)
        };
        let mut fresh = Repl::new();
        fresh.execute_document(&doc.source).expect("reparses");
        for c in doc
            .correspondence
            .iter()
            .filter(|c| c.form == Form::Literate && c.kind == DeclarationKind::Inductive)
        {
            let (arity, params, ctors) = inductive(fresh.context(), &c.logos);
            assert_eq!(
                db(&arity),
                db(&renamed(
                    &doc,
                    imported.get_global(&c.importer_key).unwrap()
                )),
                "{}",
                c.logos
            );
            assert_eq!(
                params,
                imported.inductive_num_params(&c.importer_key),
                "{}",
                c.logos
            );
            let import_ctors: Vec<(String, Term)> = imported
                .get_constructors(&c.importer_key)
                .into_iter()
                .map(|(n, t)| (entry(&doc, n).logos.clone(), db(&renamed(&doc, t))))
                .collect();
            assert_eq!(ctors, import_ctors, "{}", c.logos);
        }
    }
}

#[test]
fn a_type_that_is_not_in_type_is_not_forced_into_the_literate_form() {
    let mut ctx = Context::new();
    let holds = key(&["P", "Holds"]);
    let intro = key(&["P", "Holds", "intro"]);
    ctx.add_inductive(&holds, Term::Sort(Universe::Prop));
    ctx.add_constructor(&intro, &holds, Term::Global(holds.clone()));
    let doc = emit::emit_context(&ctx, &|_| None, &holds).unwrap_or_else(|e| panic!("{e:?}"));
    let c = entry(&doc, &holds);
    assert_eq!((c.logos.as_str(), c.form), ("P.Holds", Form::Vernacular));
    assert!(
        doc.omissions
            .iter()
            .any(|o| o.starts_with("P.Holds: ") && o.contains("`Type`")),
        "{:?}",
        doc.omissions
    );
    let mut fresh = Repl::new();
    fresh
        .execute_document(&doc.source)
        .expect("the vernacular reparses");
    assert_eq!(
        fresh.context().get_global("P.Holds"),
        Some(&Term::Sort(Universe::Prop)),
        "still a proposition"
    );
}
