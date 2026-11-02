//! W04 — actual LOGOS source for an imported Lean theorem (steps 1–7; rule R8).
//!
//! The artifact under test is the `.lg` document `interface::lean::emit` writes for a Lean
//! declaration and its dependency closure. Every test here holds that document against something
//! the emitter did not produce: a FRESH `Repl` reading the text (a new context, the prelude and
//! nothing else), the importer's own checked declarations renamed through the correspondence by
//! this file's own renamer and compared by this file's own strict α-equivalence, the independent
//! reference translator (R8), and a corruption TWIN — an export with the same names and closures in
//! which each marked declaration differs in exactly one respect.
//!
//! Required tests (the plan's list): parse and check round trip; dependent binder capture; name
//! collision; universe preservation; helper provenance; altered target type; missing dependency;
//! deterministic emission; partial-write failure. Rule lock:
//! `the_emitted_statement_matches_the_reference_translation`.

use logicaffeine_kernel::interface::lean::emit::{
    self, Correspondence, DeclarationKind, EmissionFault, EmittedDocument, Form, Origin, Published,
};
use logicaffeine_kernel::interface::lean::{
    self as adapter, reference, DiagnosticCode, ImportedContext, Verdict,
};
use logicaffeine_kernel::interface::{
    parse_command_prefix_in, print_command, Command, Repl, Surface,
};
use logicaffeine_kernel::{axiom_dependencies_of, Context, Term, Universe};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const LE_EMIT: &str = include_str!("fixtures/lean_emission/LE_Emit.ndjson");
const TWIN: &str = include_str!("fixtures/lean_emission/LE_Emit_Twin.ndjson");
const T06_002: &str = include_str!("fixtures/lean_conformance/cases_lean414/T06-002.ndjson");
const T00_001: &str = include_str!("fixtures/lean_conformance/cases_lean414/T00-001.ndjson");
const LE_NAMES: &str = include_str!("fixtures/lean_emission/LE_Names.ndjson");
const T00_014: &str = include_str!("fixtures/lean_conformance/cases_lean414/T00-014.ndjson");
const T00_019: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.ndjson");
const T04_015: &str = include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.ndjson");
const MUTUAL: &str = include_str!("fixtures/lean_arena/ordinary-mutual-polymorphic.ndjson");

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Helpers: none of them calls the emitter's own comparison.
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The importer's key for a Lean name, given by its components.
fn key(parts: &[&str]) -> String {
    parts.iter().map(|p| format!("/s:{p:?}")).collect()
}

fn le(short: &str) -> String {
    key(&["LE_Emit", short])
}

/// The production import: audited, retaining the checked body or proof of every declaration of
/// the target's dependency closure (the closure read from the export by the intake).
fn import_retaining(export: &str, target: &str) -> ImportedContext {
    let intake = adapter::intake::read(export, &adapter::intake::STAGE_A)
        .unwrap_or_else(|e| panic!("intake: {e:?}"));
    let closure = intake
        .closure(target)
        .unwrap_or_else(|e| panic!("closure of {target}: {e:?}"));
    adapter::import_retaining_closure(export, target, &closure)
        .unwrap_or_else(|e| panic!("import retaining {target}: {e:?}"))
        .0
}

fn import_plain(export: &str) -> ImportedContext {
    adapter::import_audited(export)
        .unwrap_or_else(|e| panic!("import: {e:?}"))
        .0
}

fn emit_from(imported: &ImportedContext, target: &str) -> EmittedDocument {
    emit::emit_document(imported, target).unwrap_or_else(|e| panic!("emission of {target}: {e:?}"))
}

/// A document for a target that has a body (definition or theorem).
fn emit_retaining(export: &str, target: &str) -> (ImportedContext, EmittedDocument) {
    let imported = import_retaining(export, target);
    let doc = emit_from(&imported, target);
    (imported, doc)
}

/// A document for a target with no body (an inductive or an axiom).
fn emit_plain(export: &str, target: &str) -> (ImportedContext, EmittedDocument) {
    let imported = import_plain(export);
    let doc = emit_from(&imported, target);
    (imported, doc)
}

fn entry<'a>(doc: &'a EmittedDocument, importer_key: &str) -> &'a Correspondence {
    doc.correspondence
        .iter()
        .find(|c| c.importer_key == importer_key)
        .unwrap_or_else(|| {
            panic!(
                "no correspondence entry for {importer_key}: {:#?}",
                doc.correspondence
            )
        })
}

/// A fresh toolchain reads the document: a new `Repl`, the prelude and nothing else.
fn reparse(doc: &EmittedDocument) -> Repl {
    let mut repl = Repl::new();
    repl.execute_document(&doc.source)
        .unwrap_or_else(|e| panic!("a fresh context refused the document: {e}\n{}", doc.source));
    repl
}

/// A declaration's universe telescope and type, however the context registered it.
fn header(ctx: &Context, name: &str) -> Option<(Vec<String>, Term)> {
    if let Some((levels, ty)) = ctx.poly_declaration(name) {
        return Some((levels.to_vec(), ty.clone()));
    }
    ctx.get_global(name).map(|ty| (Vec::new(), ty.clone()))
}

/// A transparent definition's body, however the context registered it.
fn body(ctx: &Context, name: &str) -> Option<Term> {
    if let Some((_, _, b)) = ctx.get_universe_poly(name) {
        return Some(b.clone());
    }
    ctx.get_definition_body(name).cloned()
}

/// Rename every constant through the document's correspondence and every universe variable through
/// `levels` — this file's renamer, not the emitter's.
fn renamed(doc: &EmittedDocument, levels: &[(String, String)], term: &Term) -> Term {
    let globals: HashMap<&str, &str> = doc
        .correspondence
        .iter()
        .map(|c| (c.importer_key.as_str(), c.logos.as_str()))
        .collect();
    let levels: HashMap<&str, &str> = levels
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    fn level(u: &Universe, levels: &HashMap<&str, &str>) -> Universe {
        match u {
            Universe::Var(v) => Universe::Var(
                levels
                    .get(v.as_str())
                    .unwrap_or_else(|| panic!("level {v}"))
                    .to_string(),
            ),
            Universe::Succ(a) => Universe::Succ(Box::new(level(a, levels))),
            Universe::Max(a, b) => {
                Universe::Max(Box::new(level(a, levels)), Box::new(level(b, levels)))
            }
            Universe::IMax(a, b) => {
                Universe::IMax(Box::new(level(a, levels)), Box::new(level(b, levels)))
            }
            other => other.clone(),
        }
    }
    fn go(t: &Term, g: &HashMap<&str, &str>, l: &HashMap<&str, &str>) -> Term {
        let b = |x: &Term| Box::new(go(x, g, l));
        match t {
            Term::Global(n) => Term::Global(
                g.get(n.as_str())
                    .unwrap_or_else(|| panic!("unmapped {n}"))
                    .to_string(),
            ),
            Term::Const { name, levels } => Term::Const {
                name: g
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("unmapped {name}"))
                    .to_string(),
                levels: levels.iter().map(|u| level(u, l)).collect(),
            },
            Term::Sort(u) => Term::Sort(level(u, l)),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: b(param_type),
                body_type: b(body_type),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: b(param_type),
                body: b(body),
            },
            Term::App(f, a) => Term::App(b(f), b(a)),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: b(discriminant),
                motive: b(motive),
                cases: cases.iter().map(|c| go(c, g, l)).collect(),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: b(body),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs.iter().map(|(n, x)| (n.clone(), go(x, g, l))).collect(),
                index: *index,
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: name.clone(),
                ty: b(ty),
                value: b(value),
                body: b(body),
            },
            Term::Var(_) | Term::Lit(_) | Term::Hole => t.clone(),
            Term::Annotated { annotation, term } => {
                Term::annotated(annotation.clone(), go(term, g, l))
            }
        }
    }
    go(term, &globals, &levels)
}

/// Binder names erased, bound variables replaced by their de Bruijn index: two terms are
/// α-equivalent exactly when their images are EQUAL — with universes compared structurally.
fn db(term: &Term) -> Term {
    fn go(t: &Term, scope: &mut Vec<String>) -> Term {
        let under = |name: &str, x: &Term, scope: &mut Vec<String>| {
            scope.push(name.to_string());
            let out = go(x, scope);
            scope.pop();
            out
        };
        match t {
            Term::Annotated { term, .. } => go(term, scope),
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
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: String::new(),
                ty: Box::new(go(ty, scope)),
                value: Box::new(go(value, scope)),
                body: Box::new(under(name, body, scope)),
            },
            Term::Fix { name, body } => Term::Fix {
                name: String::new(),
                body: Box::new(under(name, body, scope)),
            },
            Term::MutualFix { defs, index } => {
                for (n, _) in defs {
                    scope.push(n.clone());
                }
                let out = defs
                    .iter()
                    .map(|(_, b)| (String::new(), go(b, scope)))
                    .collect();
                for _ in defs {
                    scope.pop();
                }
                Term::MutualFix {
                    defs: out,
                    index: *index,
                }
            }
            Term::App(f, a) => Term::App(Box::new(go(f, scope)), Box::new(go(a, scope))),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(go(discriminant, scope)),
                motive: Box::new(go(motive, scope)),
                cases: cases.iter().map(|c| go(c, scope)).collect(),
            },
            Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {
                t.clone()
            }
        }
    }
    go(term, &mut Vec::new())
}

/// Every declaration the correspondence lists is, in a fresh context reading the document, the
/// importer's checked declaration: same universe telescope, same type, same body when it unfolds,
/// opaque when the import made it opaque.
fn assert_matches_import(imported: &Context, doc: &EmittedDocument, fresh: &Context) {
    for c in doc
        .correspondence
        .iter()
        .filter(|c| c.importer_key.is_empty())
    {
        assert_eq!(
            (c.kind, &c.origin, c.lean.as_str()),
            (DeclarationKind::Definition, &Origin::Synthesized, "")
        );
        assert!(
            body(fresh, &c.logos).is_some(),
            "{}: a constant the fresh context derived is transparent",
            c.logos
        );
        assert!(
            header(imported, &c.logos).is_none(),
            "{}: the import never had it",
            c.logos
        );
    }
    for c in doc
        .correspondence
        .iter()
        .filter(|c| !c.importer_key.is_empty())
    {
        let (fresh_levels, fresh_ty) = header(fresh, &c.logos)
            .unwrap_or_else(|| panic!("{} is not declared after reparsing", c.logos));
        let (import_levels, import_ty) =
            header(imported, &c.importer_key).expect("imported header");
        let mapped: Vec<String> = import_levels
            .iter()
            .map(|l| {
                c.levels
                    .iter()
                    .find(|(a, _)| a == l)
                    .map(|(_, b)| b.clone())
                    .expect("level mapped")
            })
            .collect();
        assert_eq!(fresh_levels, mapped, "{}: universe telescope", c.logos);
        assert_eq!(
            db(&fresh_ty),
            db(&renamed(doc, &c.levels, &import_ty)),
            "{}: type",
            c.logos
        );
        match c.kind {
            DeclarationKind::Definition => {
                let fresh_body =
                    body(fresh, &c.logos).unwrap_or_else(|| panic!("{} does not unfold", c.logos));
                let import_body = body(imported, &c.importer_key).expect("imported body");
                assert_eq!(
                    db(&fresh_body),
                    db(&renamed(doc, &c.levels, &import_body)),
                    "{}: body",
                    c.logos
                );
            }
            DeclarationKind::Theorem => {
                // Lean-theorem semantics: checked, and it unfolds to exactly the imported proof.
                let fresh_body =
                    body(fresh, &c.logos).unwrap_or_else(|| panic!("{} does not unfold", c.logos));
                let import_body =
                    body(imported, &c.importer_key).expect("the import keeps a theorem's proof");
                assert_eq!(
                    db(&fresh_body),
                    db(&renamed(doc, &c.levels, &import_body)),
                    "{}: proof",
                    c.logos
                );
            }
            DeclarationKind::Opaque | DeclarationKind::Axiom => {
                assert!(
                    body(fresh, &c.logos).is_none(),
                    "{} must not unfold after reparsing",
                    c.logos
                );
            }
            DeclarationKind::Inductive => {
                assert!(fresh.is_inductive(&c.logos), "{} is an inductive", c.logos);
                assert_eq!(
                    fresh.inductive_num_params(&c.logos),
                    imported.inductive_num_params(&c.importer_key),
                    "{}: parameters",
                    c.logos
                );
                let fresh_ctors: Vec<String> = fresh
                    .get_constructors(&c.logos)
                    .into_iter()
                    .map(|(n, _)| n.to_string())
                    .collect();
                let import_ctors: Vec<String> = imported
                    .get_constructors(&c.importer_key)
                    .into_iter()
                    .map(|(n, _)| entry(doc, n).logos.clone())
                    .collect();
                assert_eq!(
                    fresh_ctors, import_ctors,
                    "{}: constructors in order",
                    c.logos
                );
            }
            DeclarationKind::Constructor => {
                assert!(
                    fresh.is_constructor(&c.logos),
                    "{} is a constructor",
                    c.logos
                );
            }
            DeclarationKind::Quotient => {
                assert!(
                    c.logos.starts_with("kernel.quot."),
                    "{}: the kernel's own quotient identity",
                    c.logos
                );
                assert!(
                    body(fresh, &c.logos).is_none(),
                    "{} is a primitive",
                    c.logos
                );
            }
        }
    }
}

/// The source lines of one correspondence entry's declaration.
fn block(doc: &EmittedDocument, c: &Correspondence) -> String {
    let (first, last) = c
        .lines
        .unwrap_or_else(|| panic!("{} is not written", c.logos));
    doc.source
        .lines()
        .skip(first - 1)
        .take(last + 1 - first)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The document with one entry's declaration lines removed.
fn without(doc: &EmittedDocument, c: &Correspondence) -> String {
    let (first, last) = c.lines.expect("written");
    doc.source
        .lines()
        .enumerate()
        .filter(|(i, _)| *i + 1 < first || *i + 1 > last)
        .map(|(_, l)| l)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every key the LOGOS prelude declares.
fn prelude_keys() -> BTreeSet<String> {
    let ctx = logicaffeine_kernel::interface::standard_context();
    let mut keys = BTreeSet::new();
    keys.extend(ctx.iter_declarations().map(|(k, _)| k.to_string()));
    keys.extend(ctx.iter_definitions().map(|(k, _, _)| k.to_string()));
    for (k, _) in ctx.iter_inductives() {
        keys.insert(k.to_string());
        keys.extend(
            ctx.get_constructors(k)
                .into_iter()
                .map(|(c, _)| c.to_string()),
        );
    }
    keys
}

fn decline_code(outcome: Result<EmittedDocument, Verdict>) -> (DiagnosticCode, String, bool) {
    match outcome {
        Ok(doc) => panic!("expected a refusal, got a document:\n{}", doc.source),
        Err(Verdict::Decline(r)) => (r.code(), r.message().to_string(), false),
        Err(Verdict::Reject(r)) => (r.code(), r.message().to_string(), true),
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Parse and check round trip
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_definition_and_its_closure_reparse_to_the_imported_declarations() {
    let target = le("area");
    let (imported, doc) = emit_retaining(LE_EMIT, &target);
    assert_eq!(doc.target, "LE_Emit.area");
    let c = entry(&doc, &target);
    assert_eq!(
        (c.lean.as_str(), c.logos.as_str(), c.kind, &c.origin),
        (
            "LE_Emit.area",
            "LE_Emit.area",
            DeclarationKind::Definition,
            &Origin::Authored
        )
    );
    for dependency in ["Shape", "Shape.dot", "Shape.segment", "Shape.box"] {
        let k = key(&["LE_Emit"]
            .iter()
            .copied()
            .chain(dependency.split('.'))
            .collect::<Vec<_>>());
        assert_eq!(entry(&doc, &k).lean, format!("LE_Emit.{dependency}"));
    }
    let fresh = reparse(&doc);
    assert_matches_import(&imported, &doc, fresh.context());
    emit::verify(&doc).expect("the emitter's own fresh check agrees");
}

#[test]
fn a_theorem_is_written_with_its_checked_proof_and_reparses_as_leans_theorem() {
    let target = le("double_two");
    let (imported, doc) = emit_retaining(LE_EMIT, &target);
    let c = entry(&doc, &target);
    assert_eq!(
        (c.kind, c.form),
        (DeclarationKind::Theorem, Form::Vernacular)
    );
    let fresh = reparse(&doc);
    assert_matches_import(&imported, &doc, fresh.context());
    // Lean-theorem semantics: the reparsed theorem unfolds, and its body is its checked proof.
    assert!(
        body(fresh.context(), "LE_Emit.double_two").is_some(),
        "a theorem unfolds like Lean's"
    );
    assert!(
        axiom_dependencies_of(fresh.context(), "LE_Emit.double_two").is_some(),
        "the reparsed theorem carries checked proof evidence"
    );
    // The proof itself is in the document — not just the theorem's trusted name.
    let text = block(&doc, c);
    let (command, _) =
        parse_command_prefix_in(&Surface::new(), &text).expect("the theorem block parses");
    let Command::Theorem { name, proof, .. } = command else {
        panic!("not a Theorem command: {command:?}")
    };
    assert_eq!(name, "LE_Emit.double_two");
    assert!(
        !matches!(&proof, Term::Global(n) | Term::Const { name: n, .. } if n.ends_with("double_two")),
        "the written proof must be the checked proof term, not a citation of the theorem"
    );
    let (_, _, retained) = imported.retained_source().expect("retained");
    let lambdas = |t: &Term| {
        let (mut n, mut t) = (0, t);
        while let Term::Lambda { body, .. } = t {
            n += 1;
            t = body;
        }
        n
    };
    assert_eq!(
        lambdas(&proof),
        lambdas(retained),
        "the written proof has the retained proof's shape"
    );
}

#[test]
fn every_declaration_of_the_fixture_round_trips_by_itself() {
    let bodied = [
        "favorite",
        "two",
        "double",
        "area",
        "double_two",
        "pick",
        "Arrow",
        "Lift",
        "pick_self",
        "capture",
        "shadowGlobal",
        "dependent",
        "uses_oracle",
    ];
    for short in bodied {
        let target = le(short);
        let (imported, doc) = emit_retaining(LE_EMIT, &target);
        assert_eq!(entry(&doc, &target).lean, format!("LE_Emit.{short}"));
        assert_matches_import(&imported, &doc, reparse(&doc).context());
    }
    for short in ["Color", "Shape", "Chain", "Oracle", "oracle_zero"] {
        let target = le(short);
        let (imported, doc) = emit_plain(LE_EMIT, &target);
        assert_matches_import(&imported, &doc, reparse(&doc).context());
    }
    for target in [
        key(&["add"]),
        key(&["a b"]),
        key(&["a_b"]),
        key(&["x", "y"]),
        key(&["x.y"]),
        key(&["collisions"]),
    ] {
        let (imported, doc) = emit_retaining(LE_EMIT, &target);
        assert_matches_import(&imported, &doc, reparse(&doc).context());
    }
}

#[test]
fn an_original_corpus_theorem_round_trips() {
    for short in [
        "transport",
        "transportRaw",
        "jDependent",
        "symmViaRec",
        "viaMpr",
    ] {
        let target = key(&["T06_002", short]);
        let (imported, doc) = emit_retaining(T06_002, &target);
        assert_eq!(
            entry(&doc, &target).kind,
            DeclarationKind::Theorem,
            "{short}"
        );
        assert_matches_import(&imported, &doc, reparse(&doc).context());
    }
}

#[test]
fn a_theorem_resting_on_a_theorem_carries_both_proofs_and_lists_its_axioms() {
    let target = le("chained");
    let (imported, doc) = emit_retaining(LE_EMIT, &target);
    assert_eq!(entry(&doc, &le("chained")).kind, DeclarationKind::Theorem);
    assert_eq!(
        entry(&doc, &le("uses_oracle")).kind,
        DeclarationKind::Theorem,
        "a theorem stays a theorem"
    );
    assert_eq!(entry(&doc, &le("oracle_zero")).kind, DeclarationKind::Axiom);
    assert_eq!(entry(&doc, &le("Oracle")).kind, DeclarationKind::Axiom);
    assert_eq!(
        doc.assumptions,
        vec![
            "LE_Emit.Oracle".to_string(),
            "LE_Emit.oracle_zero".to_string()
        ]
    );
    let fresh = reparse(&doc);
    assert_matches_import(&imported, &doc, fresh.context());
    let usage = axiom_dependencies_of(fresh.context(), "LE_Emit.chained").expect("checked");
    assert_eq!(
        usage.axioms,
        BTreeSet::from([
            "LE_Emit.Oracle".to_string(),
            "LE_Emit.oracle_zero".to_string()
        ]),
        "the reparsed proof rests on exactly the assumptions the document lists"
    );
    assert!(usage.unresolved.is_empty());
    assert!(doc.source.contains("LE_Emit.Oracle") && doc.source.contains("## Assumptions"));
}

#[test]
fn a_mutual_polymorphic_block_is_one_checked_inductive_block() {
    for target in [
        key(&["arenaSingleton"]),
        key(&["ArenaTree", "rec"]),
        key(&["ArenaForest", "rec"]),
    ] {
        let (imported, doc) = emit_plain(MUTUAL, &target);
        let tree = entry(&doc, &key(&["ArenaTree"]));
        let forest = entry(&doc, &key(&["ArenaForest"]));
        assert_eq!(
            (tree.kind, forest.kind),
            (DeclarationKind::Inductive, DeclarationKind::Inductive)
        );
        assert_eq!(tree.lines, forest.lines, "one block, one command");
        assert_eq!(tree.levels, vec![("/s:\"u\"".to_string(), "u".to_string())]);
        for ctor in [
            ["ArenaTree", "leaf"],
            ["ArenaTree", "branch"],
            ["ArenaForest", "nil"],
            ["ArenaForest", "cons"],
        ] {
            let c = entry(&doc, &key(&ctor));
            assert_eq!(
                (c.kind, c.lines),
                (DeclarationKind::Constructor, tree.lines)
            );
        }
        let fresh = reparse(&doc);
        assert_eq!(
            fresh
                .context()
                .mutual_block_of("ArenaTree")
                .map(<[String]>::to_vec),
            Some(vec!["ArenaTree".to_string(), "ArenaForest".to_string()]),
            "the members reparse as ONE mutual block"
        );
        assert_matches_import(&imported, &doc, fresh.context());
        emit::verify(&doc).expect("verified");
    }
    let (_, doc) = emit_plain(MUTUAL, &key(&["ArenaTree", "rec"]));
    let rec = entry(&doc, &key(&["ArenaTree", "rec"]));
    assert_eq!(
        (rec.kind, &rec.origin),
        (DeclarationKind::Definition, &Origin::Synthesized)
    );
}

#[test]
fn a_constructor_that_restates_its_parameters_is_written_with_its_own_telescope() {
    // Lean's `HAdd.mk` binds `γ : outParam (Type w)` where `HAdd`'s arity says `Type w`: the constructor
    // is written with its full telescope, and reparses to exactly the imported constructor.
    for (export, target) in [
        (T00_019, key(&["T00_019", "sharedProof"])),
        (T00_019, key(&["T00_019", "viaLet"])),
        (T04_015, key(&["T04_015", "wfSum"])),
    ] {
        let (imported, doc) = emit_retaining(export, &target);
        let hadd = entry(&doc, &key(&["HAdd"]));
        assert_eq!(
            (hadd.kind, hadd.form),
            (DeclarationKind::Inductive, Form::Vernacular)
        );
        let text = block(&doc, hadd);
        assert!(
            text.contains("outParam"),
            "the restated telescope keeps `outParam`:\n{text}"
        );
        assert!(
            doc.omissions
                .iter()
                .any(|o| o.starts_with(&format!("{}: ", hadd.logos))),
            "{:?}",
            doc.omissions
        );
        let fresh = reparse(&doc);
        assert_matches_import(&imported, &doc, fresh.context());
        let ctor = entry(&doc, &key(&["HAdd", "mk"]));
        let (_, fresh_ty) = header(fresh.context(), &ctor.logos).unwrap();
        let (_, import_ty) = header(&imported, &ctor.importer_key).unwrap();
        assert_eq!(
            db(&fresh_ty),
            db(&renamed(&doc, &ctor.levels, &import_ty)),
            "HAdd.mk exactly"
        );
        let reference = reference::reference_declaration(export, &target).expect("reference");
        emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{target}: {f}"));
    }
}

#[test]
fn a_lean_structure_is_written_with_its_projections_and_eta_before_its_recursor() {
    let mut structures = 0;
    for (export, target) in [
        (LE_EMIT, le("two")),
        (LE_EMIT, le("double")),
        (LE_EMIT, le("Lift")),
        (T06_002, key(&["T06_002", "transport"])),
        (T00_019, key(&["T00_019", "sharedProof"])),
    ] {
        let (imported, doc) = emit_retaining(export, &target);
        let fresh = reparse(&doc);
        assert_matches_import(&imported, &doc, fresh.context());
        for ind in doc
            .correspondence
            .iter()
            .filter(|c| c.kind == DeclarationKind::Inductive && c.lines.is_some())
        {
            let Some(info) = imported.struct_info(&ind.importer_key) else {
                assert!(
                    fresh.context().struct_info(&ind.logos).is_none(),
                    "{}: only a Lean structure gains η",
                    ind.logos
                );
                continue;
            };
            structures += 1;
            let written = fresh
                .context()
                .struct_info(&ind.logos)
                .unwrap_or_else(|| panic!("{} lost its η", ind.logos));
            assert_eq!(
                written.projections.len(),
                info.projections.len(),
                "{}",
                ind.logos
            );
            for (i, (projection, logos)) in info
                .projections
                .iter()
                .zip(&written.projections)
                .enumerate()
            {
                let c = entry(&doc, projection);
                assert_eq!(c.logos, *logos, "{projection}");
                assert_eq!(
                    c.logos,
                    format!("{}.proj_{}", ind.logos, i + 1),
                    "a readable projection name"
                );
                assert_eq!(
                    (c.kind, &c.origin, c.lines),
                    (DeclarationKind::Definition, &Origin::Synthesized, ind.lines)
                );
                assert_eq!(
                    c.lean, *projection,
                    "a reader projection has no Lean name and does not pretend to"
                );
            }
            // In the document: the block, then its projections, then `Structure`, all before the recursor.
            let text = block(&doc, ind);
            assert!(text.contains(&format!("Structure {}", ind.logos)), "{text}");
            let rec = format!("{}/s:\"rec\"", ind.importer_key);
            if let Some(r) = doc.correspondence.iter().find(|c| c.importer_key == rec) {
                assert!(
                    r.lines.unwrap().0 > ind.lines.unwrap().1,
                    "{} is written after its η",
                    r.logos
                );
            }
        }
        let reference = reference::reference_declaration(export, &target).expect("reference");
        emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{target}: {f}"));
    }
    assert!(
        structures > 0,
        "the closures above contain Lean structures (OfNat, PProd, ULift, …)"
    );
}

#[test]
fn quotients_are_written_as_the_kernel_primitives_they_are() {
    let target = key(&["T00_014", "needsFunext"]);
    let (imported, doc) = emit_retaining(T00_014, &target);
    let quotients: Vec<&Correspondence> = doc
        .correspondence
        .iter()
        .filter(|c| c.kind == DeclarationKind::Quotient)
        .collect();
    let names: BTreeSet<&str> = quotients.iter().map(|c| c.logos.as_str()).collect();
    assert!(names.contains("kernel.quot.type"), "{names:?}");
    for q in &quotients {
        assert_eq!(
            (q.logos.as_str(), q.form),
            (q.importer_key.as_str(), Form::Vernacular)
        );
        assert!(
            block(&doc, q).starts_with("Quotient "),
            "{}",
            block(&doc, q)
        );
        assert!(
            !doc.assumptions.contains(&q.logos),
            "a quotient primitive is not an axiom of the document"
        );
    }
    let fresh = reparse(&doc);
    assert_matches_import(&imported, &doc, fresh.context());
    let reference = reference::reference_declaration(T00_014, &target).expect("reference");
    emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{f}"));
}

#[test]
fn a_value_the_import_did_not_retain_is_refused_never_assumed() {
    // A Lean theorem unfolds, so its proof is in the context and needs no retention.
    let imported = import_plain(LE_EMIT);
    for short in ["double_two", "uses_oracle", "pick_self"] {
        let doc = emit_from(&imported, &le(short));
        assert_eq!(
            entry(&doc, &le(short)).kind,
            DeclarationKind::Theorem,
            "{short}"
        );
        assert_matches_import(&imported, &doc, reparse(&doc).context());
    }
    // A sealed `opaque` keeps no body: without its retained value it is refused, never assumed.
    let mut ctx = nat_context();
    let sealed = key(&["C", "sealed"]);
    ctx.declare_opaque_checked(&sealed, Vec::new(), nat(), Term::Global("Zero".into()))
        .expect("checks");
    let (code, message, rejected) = decline_code(emit::emit_context(&ctx, &|_| None, &sealed));
    assert!(
        !rejected,
        "a missing value is a limitation, not an invalid input: {message}"
    );
    assert_eq!(code, DiagnosticCode::UnsupportedFeature);
    assert!(
        message.contains("never written as an assumption"),
        "{message}"
    );
    // Given its value, it is written as an `Opaque` and reparses sealed.
    let value = |k: &str| (k == sealed).then(|| Term::Global("Zero".into()));
    let doc = emit::emit_context(&ctx, &value, &sealed).unwrap_or_else(|e| panic!("{e:?}"));
    let c = entry(&doc, &sealed);
    assert_eq!(
        (c.logos.as_str(), c.kind),
        ("C.sealed", DeclarationKind::Opaque)
    );
    assert!(block(&doc, c).starts_with("Opaque "), "{}", block(&doc, c));
    let fresh = reparse(&doc);
    assert!(
        body(fresh.context(), "C.sealed").is_none(),
        "an opaque stays sealed"
    );
    assert!(
        axiom_dependencies_of(fresh.context(), "C.sealed").is_some(),
        "and checked"
    );
    assert!(
        doc.assumptions.is_empty(),
        "a checked value is not an assumption"
    );
}

#[test]
fn the_document_is_literate_logos_not_a_lean_dump() {
    let (_, doc) = emit_retaining(LE_EMIT, &le("double_two"));
    assert!(doc
        .source
        .starts_with("/-!\n# LOGOS source for the Lean declaration `LE_Emit.double_two`"));
    for heading in [
        "## Assumptions",
        "## Names",
        "## Not carried",
        "/-! ## Types -/",
        "/-! ## The target -/",
    ] {
        assert!(doc.source.contains(heading), "missing {heading:?}");
    }
    for line in doc.source.lines() {
        for lean in [
            "theorem ",
            "def ",
            "namespace ",
            "universe ",
            "inductive ",
            "#check",
            "import ",
        ] {
            assert!(
                !line.starts_with(lean),
                "a Lean command leaked into the document: {line}"
            );
        }
    }
    // Every written declaration is preceded by a prose line naming the Lean declaration it carries.
    // A unit's own entry (not its constructors, not the reader's structure projections and not the
    // constants the fresh context derives, which share its lines) is named by the prose above it.
    let projection = |c: &Correspondence| c.importer_key.starts_with("arena.structure.projection.");
    for c in doc.correspondence.iter().filter(|c| {
        c.lines.is_some()
            && c.kind != DeclarationKind::Constructor
            && !c.importer_key.is_empty()
            && !projection(c)
    }) {
        let (first, _) = c.lines.unwrap();
        let above = doc.source.lines().nth(first - 2).expect("a line above");
        assert!(
            above.starts_with("/-- ") && above.ends_with(" -/"),
            "{}: {above}",
            c.logos
        );
        assert!(
            above.contains(&format!("`{}`", c.lean)),
            "{}: the prose names {}: {above}",
            c.logos,
            c.lean
        );
    }
    // The whole document is commands the text-only door reads: prose, or a declaration.
    let (mut offset, mut declarations) = (0, 0);
    while !doc.source[offset..].trim().is_empty() {
        let (command, n) =
            parse_command_prefix_in(&Surface::new(), &doc.source[offset..]).expect("parses");
        if !matches!(command, Command::Comment) {
            declarations += 1;
        }
        offset += n;
    }
    // One command per written unit, plus, for a structure, one per projection and its `Structure`.
    let written: BTreeSet<(usize, usize)> =
        doc.correspondence.iter().filter_map(|c| c.lines).collect();
    let projections = doc.correspondence.iter().filter(|c| projection(c)).count();
    let structures: BTreeSet<(usize, usize)> = doc
        .correspondence
        .iter()
        .filter(|c| projection(c))
        .filter_map(|c| c.lines)
        .collect();
    assert_eq!(
        declarations,
        written.len() + projections + structures.len(),
        "the document's commands"
    );
}

#[test]
fn trailing_junk_and_smuggled_declarations_are_refused() {
    let (_, doc) = emit_retaining(LE_EMIT, &le("two"));
    emit::verify(&doc).expect("clean");
    let mut noted = doc.clone();
    noted
        .source
        .push_str("\n-- a reader's note declares nothing\n");
    emit::verify(&noted).expect("prose after the document is harmless");

    let mut junk = doc.clone();
    junk.source.push_str("\nThis sentence is not LOGOS.\n");
    assert!(
        matches!(emit::verify(&junk), Err(EmissionFault::Reparse { .. })),
        "trailing junk"
    );

    let mut unterminated = doc.clone();
    unterminated.source.push_str("\n/-- an unclosed comment\n");
    assert!(
        matches!(
            emit::verify(&unterminated),
            Err(EmissionFault::Reparse { .. })
        ),
        "unconsumed suffix"
    );

    let cheat = print_command(&Command::Axiom {
        name: "cheat".into(),
        ty: Term::Global("False".into()),
    })
    .expect("an axiom has a spelling");
    let mut smuggled = doc.clone();
    smuggled.source.push_str(&format!("\n{cheat}\n"));
    assert!(
        matches!(
            emit::verify(&smuggled),
            Err(EmissionFault::CommandCount { .. } | EmissionFault::Undeclared { .. })
        ),
        "an assumption the correspondence does not list is refused: {:?}",
        emit::verify(&smuggled).err()
    );
}

#[test]
fn every_binder_annotation_the_document_drops_is_recorded() {
    use logicaffeine_kernel::elaborate::ParamKind;
    let (imported, doc) = emit_retaining(LE_EMIT, &le("pick_self"));
    for c in &doc.correspondence {
        let annotated = imported
            .binder_kinds(&c.importer_key)
            .is_some_and(|kinds| kinds.iter().any(|k| *k != ParamKind::Explicit));
        let recorded = doc.losses.iter().any(|(category, detail)| {
            category == "binder_annotations" && detail.starts_with(&format!("`{}`:", c.logos))
        });
        assert_eq!(
            annotated, recorded,
            "{}: a dropped annotation is recorded, and only a dropped one",
            c.logos
        );
    }
    assert!(
        doc.losses
            .iter()
            .any(|(_, d)| d.starts_with("`LE_Emit.pick`:")),
        "pick's α and β are implicit in Lean"
    );
    // The document's bytes depend on the checked declarations alone (a replay rebuilds them from a
    // certificate that carries no binder metadata): the prose states the loss, never the import's list.
    assert!(doc.source.contains(
        "Binder annotations (implicit, strict-implicit and instance binders) are not written"
    ));
    assert!(!doc.source.contains("- `LE_Emit.pick`"));
}

#[test]
fn every_constant_the_reparsed_document_introduces_is_listed() {
    let (_, doc) = emit_retaining(LE_EMIT, &le("area"));
    let fresh = reparse(&doc);
    let prelude = prelude_keys();
    let ctx = fresh.context();
    let mut introduced: BTreeSet<String> = BTreeSet::new();
    introduced.extend(ctx.iter_declarations().map(|(k, _)| k.to_string()));
    introduced.extend(ctx.iter_definitions().map(|(k, _, _)| k.to_string()));
    for (k, _) in ctx.iter_inductives() {
        introduced.insert(k.to_string());
        introduced.extend(
            ctx.get_constructors(k)
                .into_iter()
                .map(|(c, _)| c.to_string()),
        );
    }
    let listed: BTreeSet<&str> = doc
        .correspondence
        .iter()
        .map(|c| c.logos.as_str())
        .collect();
    for key in introduced.iter().filter(|k| !prelude.contains(*k)) {
        assert!(
            listed.contains(key.as_str()),
            "{key} is introduced by the document and not listed"
        );
    }
    // The derived constants are listed against the declaration that produced them.
    let shape = entry(&doc, &le("Shape"));
    let derived: Vec<&Correspondence> = doc
        .correspondence
        .iter()
        .filter(|c| c.importer_key.is_empty())
        .collect();
    assert!(derived.iter().any(|c| c.logos == "LE_Emit.Shape_casesOn"
        && c.lines == shape.lines
        && c.form == shape.form));
    // Dropping one from the correspondence, or listing one the context never derived, is refused.
    let mut unlisted = doc.clone();
    let dropped = unlisted
        .correspondence
        .iter()
        .position(|c| c.logos == "LE_Emit.Shape_casesOn")
        .unwrap();
    unlisted.correspondence.remove(dropped);
    assert_eq!(
        emit::verify(&unlisted).err(),
        Some(EmissionFault::Undeclared {
            logos: "LE_Emit.Shape_casesOn".into()
        })
    );
    let mut invented = doc.clone();
    let mut phantom = derived[0].clone();
    phantom.logos = "LE_Emit.Shape_phantom".into();
    invented.correspondence.push(phantom);
    assert_eq!(
        emit::verify(&invented).err(),
        Some(EmissionFault::Missing {
            logos: "LE_Emit.Shape_phantom".into()
        })
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Dependent binder capture
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn dependent_binders_reparse_uncaptured() {
    for short in ["capture", "shadowGlobal", "dependent"] {
        let target = le(short);
        let (imported, doc) = emit_retaining(LE_EMIT, &target);
        assert_matches_import(&imported, &doc, reparse(&doc).context());
    }
}

fn nat() -> Term {
    Term::Global("Nat".into())
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

/// A context holding Nat under the importer's identities, as an import of Lean's `Nat` does.
fn nat_context() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", nat());
    ctx.add_constructor("Succ", "Nat", pi("_", nat(), nat()));
    ctx
}

/// Binders named exactly like the constants the body refers to — a printer that kept them would
/// capture the reference and write a different function.
fn adversarial_context(identity_instead: bool) -> Context {
    let mut ctx = nat_context();
    let k = key(&["C", "k"]);
    let k_body = if identity_instead {
        lam("x", nat(), Term::Var("x".into()))
    } else {
        lam("Zero", nat(), Term::Global("Zero".into()))
    };
    ctx.declare(&k, pi("Zero", nat(), nat()), k_body)
        .expect("C.k checks");
    let user = key(&["C", "user"]);
    let user_body = lam(
        "k",
        nat(),
        app(Term::Global(k.clone()), Term::Var("k".into())),
    );
    ctx.declare(&user, pi("k", nat(), nat()), user_body)
        .expect("C.user checks");
    let shadow = key(&["C", "shadow"]);
    let shadow_body = lam(
        "x",
        nat(),
        lam(
            "x",
            nat(),
            app(Term::Global("Succ".into()), Term::Var("x".into())),
        ),
    );
    ctx.declare(&shadow, pi("x", nat(), pi("x", nat(), nat())), shadow_body)
        .expect("C.shadow checks");
    let user2 = key(&["C", "both"]);
    let both_body = app(
        Term::Global(user.clone()),
        app(
            app(Term::Global(shadow.clone()), Term::Global("Zero".into())),
            Term::Global("Zero".into()),
        ),
    );
    ctx.declare(&user2, nat(), both_body)
        .expect("C.both checks");
    ctx
}

#[test]
fn a_binder_named_like_a_constant_never_captures_it() {
    let ctx = adversarial_context(false);
    let target = key(&["C", "both"]);
    let doc = emit::emit_context(&ctx, &|_| None, &target).unwrap_or_else(|e| panic!("{e:?}"));
    let fresh = reparse(&doc);
    assert_matches_import(&ctx, &doc, fresh.context());
    // C.k ignores its argument and returns the constructor Zero; C.shadow returns the successor of
    // its SECOND argument; C.user passes its argument to C.k.
    assert_eq!(
        db(&body(fresh.context(), "C.k").unwrap()),
        db(&lam("n", nat(), Term::Global("Zero".into())))
    );
    assert_eq!(
        db(&body(fresh.context(), "C.shadow").unwrap()),
        db(&lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                app(Term::Global("Succ".into()), Term::Var("b".into()))
            )
        ))
    );
    assert_eq!(
        db(&body(fresh.context(), "C.user").unwrap()),
        db(&lam(
            "n",
            nat(),
            app(Term::Global("C.k".into()), Term::Var("n".into()))
        ))
    );
    // The captured reading — C.k as the identity — is a different declaration, and presenting it as
    // this emission is refused naming C.k's body.
    let captured =
        emit::emit_context(&adversarial_context(true), &|_| None, &target).expect("emits");
    let mut forged = doc.clone();
    forged.source = captured.source.clone();
    assert_eq!(
        emit::verify(&forged).err(),
        Some(EmissionFault::Body {
            logos: "C.k".into()
        })
    );
}

/// A reference is written so that the namespace its declaration elaborates in cannot capture it — a
/// universe-polymorphic reference (`c.{u}`) as much as a plain one, and whether what would capture it
/// is the declaration itself or a universe-polymorphic neighbour.
#[test]
fn a_reference_is_written_so_the_declarations_own_namespace_cannot_capture_it() {
    // Corpus case T03-023 (harvested original): `instDecidableEqWithInt` rests on Lean's `Int.decEq`, whose body
    // calls the root `decEq.{u}` from inside `Int.decEq`'s own namespace — where `decEq` means `Int.decEq`.
    let t03_023 = include_str!("fixtures/lean_conformance/cases_lean414/harvest/T03-023.ndjson");
    let target = key(&["T03_023", "instDecidableEqWithInt"]);
    let (imported, doc) = emit_retaining(t03_023, &target);
    let fresh = reparse(&doc);
    assert_matches_import(&imported, &doc, fresh.context());
    let reference = reference::reference_declaration(t03_023, &target).expect("reference");
    emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{f}"));
    let dec_eq = entry(&doc, &key(&["Int", "decEq"]));
    assert!(
        block(&doc, dec_eq).contains("_root_.decEq.{"),
        "{}",
        block(&doc, dec_eq)
    );

    // Both captures from Lean itself: `LE_Names.pick` calls the root `pick.{u}`, and `LE_Names.useTwice`
    // calls the root `twice` beside the universe-polymorphic `LE_Names.twice`.
    for (short, written) in [("pick", "_root_.pick.{"), ("useTwice", "_root_.twice")] {
        let target = key(&["LE_Names", short]);
        let (imported, doc) = emit_retaining(LE_NAMES, &target);
        let fresh = reparse(&doc);
        assert_matches_import(&imported, &doc, fresh.context());
        let reference = reference::reference_declaration(LE_NAMES, &target).expect("reference");
        emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{short}: {f}"));
        assert!(
            block(&doc, entry(&doc, &target)).contains(written),
            "{}",
            block(&doc, entry(&doc, &target))
        );
    }

    // The same two captures, built by hand. `pick.{u}` is a root constant `N.pick` calls; `N.twice.{u}`
    // is a universe-polymorphic neighbour of the root `twice` that `N.useTwice` calls.
    let mut ctx = nat_context();
    let sort_u = Term::Sort(Universe::Var("u".into()));
    let identity = lam(
        "α",
        sort_u.clone(),
        lam("a", Term::Var("α".into()), Term::Var("a".into())),
    );
    let identity_ty = pi(
        "α",
        sort_u,
        pi("_", Term::Var("α".into()), Term::Var("α".into())),
    );
    let at_nat = |name: String| Term::Const {
        name,
        levels: vec![Universe::Type(0)],
    };
    ctx.declare_universe_poly(
        &key(&["pick"]),
        vec!["u".into()],
        identity_ty.clone(),
        identity.clone(),
    )
    .expect("pick");
    let n_pick = key(&["N", "pick"]);
    let n_pick_body = lam(
        "n",
        nat(),
        app(app(at_nat(key(&["pick"])), nat()), Term::Var("n".into())),
    );
    ctx.declare(&n_pick, pi("_", nat(), nat()), n_pick_body)
        .expect("N.pick");
    ctx.declare_universe_poly(
        &key(&["N", "twice"]),
        vec!["u".into()],
        identity_ty,
        identity,
    )
    .expect("N.twice");
    let twice_body = lam(
        "n",
        nat(),
        app(
            Term::Global("Succ".into()),
            app(Term::Global("Succ".into()), Term::Var("n".into())),
        ),
    );
    ctx.declare(&key(&["twice"]), pi("_", nat(), nat()), twice_body)
        .expect("twice");
    let use_twice = key(&["N", "useTwice"]);
    ctx.declare(
        &use_twice,
        pi("_", nat(), nat()),
        lam(
            "n",
            nat(),
            app(Term::Global(key(&["twice"])), Term::Var("n".into())),
        ),
    )
    .expect("N.useTwice");
    let both = key(&["N", "both"]);
    let both_body = app(
        Term::Global(n_pick.clone()),
        app(
            Term::Global(use_twice.clone()),
            app(
                app(at_nat(key(&["N", "twice"])), nat()),
                Term::Global("Zero".into()),
            ),
        ),
    );
    ctx.declare(&both, nat(), both_body).expect("N.both");

    let doc = emit::emit_context(&ctx, &|_| None, &both).unwrap_or_else(|e| panic!("{e:?}"));
    let fresh = reparse(&doc);
    assert_matches_import(&ctx, &doc, fresh.context());
    assert!(
        block(&doc, entry(&doc, &n_pick)).contains("_root_.pick.{"),
        "{}",
        block(&doc, entry(&doc, &n_pick))
    );
    assert!(
        block(&doc, entry(&doc, &use_twice)).contains("_root_.twice"),
        "{}",
        block(&doc, entry(&doc, &use_twice))
    );
    emit::verify(&doc).expect("verified");
}

/// The binder names of a term's outer Π (or λ) telescope.
fn telescope_names(t: &Term) -> Vec<String> {
    let mut out = Vec::new();
    let mut t = t;
    loop {
        match t {
            Term::Pi {
                param,
                body_type: b,
                ..
            }
            | Term::Lambda { param, body: b, .. } => {
                out.push(param.clone());
                t = b;
            }
            _ => return out,
        }
    }
}

#[test]
fn lean_binder_names_are_kept_unless_keeping_one_changes_the_meaning() {
    // No collision: T06-002 `transport {α : Type} (P : α → Prop) {a b : α} (h : a = b) (pa : P a)` keeps
    // every Lean binder name, in its statement and in its proof, and records no rename.
    let target = key(&["T06_002", "transport"]);
    let (_, doc) = emit_retaining(T06_002, &target);
    let fresh = reparse(&doc);
    // The statement's non-dependent binders (`h`, `pa`) are written as arrows, which name nothing.
    let (_, ty) = header(fresh.context(), "T06_002.transport").unwrap();
    assert_eq!(telescope_names(&ty), ["α", "P", "a", "b", "_", "_"]);
    let proof = body(fresh.context(), "T06_002.transport").unwrap();
    assert_eq!(telescope_names(&proof), ["α", "P", "a", "b", "h", "pa"]);
    // The one binder of `transport` that is not kept is the proof's hygienic `h✝` inside the motive,
    // which sits under the statement's own `h`: it is written `h1`, and the rename is recorded.
    let transport: Vec<&str> = doc
        .losses
        .iter()
        .filter(|(c, d)| c == "binder_renamed" && d.starts_with("`T06_002.transport`"))
        .map(|(_, d)| d.as_str())
        .collect();
    assert_eq!(
        transport,
        ["`T06_002.transport`: the Lean binder `h` is written `h1` because an enclosing binder already has that name"]
    );
    // Lean's hygienic binders (`a✝`, `h✝`) keep their user-facing names, as `eraseMacroScopes` reads them.
    assert!(
        !doc.source.contains("_hyg"),
        "no macro scope reaches the text"
    );
    assert!(
        doc.source.contains("eraseMacroScopes"),
        "the Names section says hygienic names were erased"
    );

    // Collisions: the Lean name steps to the first free `name1`, `name2`, … and every step is recorded,
    // with its reason, in the losses and in the document's Names section.
    let lean = |depth: usize, name: &str| format!("arena.local.{depth}.{name}");
    let mut ctx = nat_context();
    let k = key(&["C", "k"]);
    ctx.declare(
        &k,
        pi("_", nat(), nat()),
        lam(&lean(0, "n"), nat(), Term::Var(lean(0, "n"))),
    )
    .expect("C.k");
    // `k` would capture the constant `C.k`, which the body writes as `k`.
    let user = key(&["C", "user"]);
    let user_body = lam(
        &lean(0, "k"),
        nat(),
        app(Term::Global(k.clone()), Term::Var(lean(0, "k"))),
    );
    ctx.declare(&user, pi(&lean(0, "k"), nat(), nat()), user_body)
        .expect("C.user");
    // The inner `x` would shadow the outer one; `fun` reads as syntax.
    let shadow = key(&["C", "shadow"]);
    let shadow_body = lam(
        &lean(0, "x"),
        nat(),
        lam(
            &lean(1, "x"),
            nat(),
            lam(
                &lean(2, "fun"),
                nat(),
                app(Term::Global("Succ".into()), Term::Var(lean(1, "x"))),
            ),
        ),
    );
    let shadow_ty = pi("_", nat(), pi("_", nat(), pi("_", nat(), nat())));
    ctx.declare(&shadow, shadow_ty, shadow_body)
        .expect("C.shadow");
    let both = key(&["C", "both"]);
    let zero = || Term::Global("Zero".into());
    let both_body = app(
        Term::Global(user.clone()),
        app(
            app(app(Term::Global(shadow.clone()), zero()), zero()),
            zero(),
        ),
    );
    ctx.declare(&both, nat(), both_body).expect("C.both");

    let doc = emit::emit_context(&ctx, &|_| None, &both).unwrap_or_else(|e| panic!("{e:?}"));
    let fresh = reparse(&doc);
    assert_eq!(
        telescope_names(&body(fresh.context(), "C.user").unwrap()),
        ["k1"]
    );
    assert_eq!(
        telescope_names(&body(fresh.context(), "C.shadow").unwrap()),
        ["x", "x1", "n"]
    );
    assert_eq!(
        telescope_names(&body(fresh.context(), "C.k").unwrap()),
        ["n"],
        "no collision, no rename"
    );
    let renamed: Vec<&str> = doc
        .losses
        .iter()
        .filter(|(c, _)| c == "binder_renamed")
        .map(|(_, d)| d.as_str())
        .collect();
    assert_eq!(
        renamed,
        [
            "`C.shadow`: the Lean binder `x` is written `x1` because an enclosing binder already has that name",
            "`C.shadow`: the Lean binder `fun` is written `n` because the term surface reads that name as syntax",
            "`C.user`: the Lean binder `k` is written `k1` because a constant written in this document has that name",
        ]
    );
    for line in &renamed {
        assert!(
            doc.source.contains(&format!("- {line}")),
            "the Names section states it: {line}"
        );
    }
    // The renamed document still means the imported declarations exactly.
    assert_matches_import(&ctx, &doc, fresh.context());
}

/// Every name a binder of `t` is written with: every λ, `let` and `fix`, and every Π the text names
/// (an arrow's `_` names nothing).
fn binder_names(t: &Term, out: &mut BTreeSet<String>) {
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Pi {
                param,
                param_type,
                body_type: b,
            }
            | Term::Lambda {
                param,
                param_type,
                body: b,
            } => {
                if param != "_" {
                    out.insert(param.clone());
                }
                work.push(param_type);
                work.push(b);
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                out.insert(name.clone());
                work.extend([&**ty, &**value, &**body]);
            }
            Term::Fix { name, body } => {
                out.insert(name.clone());
                work.push(body);
            }
            Term::MutualFix { defs, .. } => {
                for (name, b) in defs {
                    out.insert(name.clone());
                    work.push(b);
                }
            }
            Term::App(f, a) => work.extend([&**f, &**a]),
            Term::Annotated { term, .. } => work.push(term),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.extend([&**discriminant, &**motive]);
                work.extend(cases.iter());
            }
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
        }
    }
}

/// The binder names of one reparsed declaration: its type, and its body when it has one.
fn declaration_binders(fresh: &Context, logos: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some((_, ty)) = header(fresh, logos) {
        binder_names(&ty, &mut out);
    }
    if let Some(b) = body(fresh, logos) {
        binder_names(&b, &mut out);
    }
    out
}

/// What a document says about Lean's binder names is true of its text, and every binder it writes
/// reads as that binder alone.
///
/// - Every recorded rename (`binder_renamed`) names a binder the reparsed unit really carries under the
///   written name, and the Names section lists exactly the recorded renames.
/// - No binder of a written declaration is named like a constant the LOGOS prelude declares, or like a
///   component of a name the document writes, and no Lean macro scope reaches the text.
fn assert_binder_names_are_honest(doc: &EmittedDocument, fresh: &Context) {
    let prelude = prelude_keys();
    let written: Vec<&Correspondence> = doc
        .correspondence
        .iter()
        .filter(|c| c.lines.is_some() && !c.importer_key.is_empty())
        .collect();
    let constants: BTreeSet<&str> = written.iter().flat_map(|c| c.logos.split('.')).collect();
    let mut renames = Vec::new();
    for (category, detail) in &doc.losses {
        if category != "binder_renamed" {
            continue;
        }
        // "`D`: the Lean binder `L` is written `W` because …"
        let ticked: Vec<&str> = detail.split('`').collect();
        assert!(
            ticked.len() >= 7 && ticked[2] == ": the Lean binder " && ticked[4] == " is written ",
            "a rename record reads `D`: the Lean binder `L` is written `W` because …: {detail}"
        );
        let (declared, lean, spelled) = (ticked[1], ticked[3], ticked[5]);
        let lines = written
            .iter()
            .find(|c| c.logos == declared)
            .and_then(|c| c.lines)
            .unwrap_or_else(|| {
                panic!("the rename record names `{declared}`, which the document does not write")
            });
        let unit: BTreeSet<String> = written
            .iter()
            .filter(|c| c.lines == Some(lines))
            .flat_map(|c| declaration_binders(fresh, &c.logos))
            .collect();
        assert!(
            unit.contains(spelled),
            "`{declared}` records its Lean binder `{lean}` as written `{spelled}`, but no binder of it is written \
             `{spelled}` (its binders: {unit:?})"
        );
        assert!(
            doc.source.contains(&format!("\n- {detail}\n")),
            "the Names section states it: {detail}"
        );
        renames.push(detail);
    }
    let listed = doc
        .source
        .lines()
        .filter(|l| l.starts_with("- `") && l.contains("`: the Lean binder `"))
        .count();
    assert_eq!(
        listed,
        renames.len(),
        "the Names section lists exactly the recorded renames"
    );
    for c in &written {
        for name in declaration_binders(fresh, &c.logos) {
            assert!(
                !prelude.contains(&name),
                "{}: the binder `{name}` reads as the prelude's constant `{name}`",
                c.logos
            );
            assert!(
                !constants.contains(name.as_str()),
                "{}: the binder `{name}` reads as a constant of the document",
                c.logos
            );
            assert!(
                !name.contains("_hyg") && !name.contains("_@"),
                "{}: a Lean macro scope reached the text: {name}",
                c.logos
            );
        }
    }
}

/// A Lean binder named like a constant the LOGOS prelude declares would read as that constant, so it
/// steps to the first free `name1` and the rename is recorded with that reason — whether or not the
/// document writes the constant (when it does, the document's own reason is given).
#[test]
fn a_binder_named_like_a_prelude_constant_is_renamed_and_recorded() {
    // `def keepsNothing (le : Nat) (add : Nat) (Nat : Bool) : Bool := Nat`
    let target = key(&["LE_Names", "keepsNothing"]);
    let (imported, doc) = emit_retaining(LE_NAMES, &target);
    let fresh = reparse(&doc);
    assert_matches_import(&imported, &doc, fresh.context());
    let reference = reference::reference_declaration(LE_NAMES, &target).expect("reference");
    emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(
        telescope_names(&body(fresh.context(), "LE_Names.keepsNothing").unwrap()),
        ["le1", "add1", "Nat1"]
    );
    let renamed: Vec<&str> = doc
        .losses
        .iter()
        .filter(|(c, _)| c == "binder_renamed")
        .map(|(_, d)| d.as_str())
        .collect();
    assert_eq!(
        renamed,
        [
            "`LE_Names.keepsNothing`: the Lean binder `le` is written `le1` because the LOGOS prelude declares a constant of that name",
            "`LE_Names.keepsNothing`: the Lean binder `add` is written `add1` because the LOGOS prelude declares a constant of that name",
            "`LE_Names.keepsNothing`: the Lean binder `Nat` is written `Nat1` because a constant written in this document has that name",
        ]
    );
    assert_binder_names_are_honest(&doc, fresh.context());
}

/// `Type1` reads as the universe `Type 1`, not as a name: a Lean binder called that cannot be written
/// under its own name, and the name it is written under is recorded — never chosen silently by the
/// printer.
#[test]
fn a_lean_binder_named_like_a_sort_is_renamed_and_recorded() {
    let lean = |depth: usize, name: &str| format!("arena.local.{depth}.{name}");
    let mut ctx = nat_context();
    let k = key(&["C", "k"]);
    ctx.declare(
        &k,
        pi("_", nat(), nat()),
        lam(&lean(0, "Type1"), nat(), Term::Var(lean(0, "Type1"))),
    )
    .expect("C.k");
    let doc = emit::emit_context(&ctx, &|_| None, &k).unwrap_or_else(|e| panic!("{e:?}"));
    let fresh = reparse(&doc);
    assert_matches_import(&ctx, &doc, fresh.context());
    let written = telescope_names(&body(fresh.context(), "C.k").unwrap());
    assert_eq!(written.len(), 1, "{written:?}");
    assert_ne!(written[0], "Type1");
    let renamed: Vec<String> = doc
        .losses
        .iter()
        .filter(|(c, _)| c == "binder_renamed")
        .map(|(_, d)| d.clone())
        .collect();
    assert_eq!(
        renamed,
        [format!(
            "`C.k`: the Lean binder `Type1` is written `{}` because the term surface reads that name as syntax",
            written[0]
        )]
    );
    assert_binder_names_are_honest(&doc, fresh.context());
}

#[test]
fn the_corpus_binder_case_writes_every_lean_name_that_still_reads_as_itself() {
    // T00-001, the corpus's "de Bruijn index → named variable" case. An export's binder names are hints, and the
    // exporter shares subterms up to α: `shadowedInnermost`'s innermost `fun x => x` arrives as
    // `identityNat`'s `fun a => a`, so its Lean name there IS `a`.
    let cases: [(&str, &[&str], &[&str]); 7] = [
        ("identityNat", &["a"], &[]),
        ("pickOuter", &["a", "b"], &[]),
        (
            "shadowedInnermost",
            &["x", "x1", "a"],
            &["the Lean binder `x` is written `x1` because an enclosing binder already has that name"],
        ),
        ("shadowedOutermost", &["x", "_y", "_z"], &[]),
        ("usesSectionVar", &["k"], &[]),
        // `Nat` is the prelude's own inductive: as a binder it would read as that constant.
        (
            "binderNamedLikeAGlobal",
            &["Nat1"],
            &["the Lean binder `Nat` is written `Nat1` because the LOGOS prelude declares a constant of that name"],
        ),
        // Its value is the type `(n : Nat) -> Fin n -> Nat`: nothing refers to the inner `n`, so it is an
        // arrow and carries no name — neither kept nor renamed.
        ("innerTypeMentionsOuter", &["n", "_"], &[]),
    ];
    for (short, names, renames) in cases {
        let target = key(&["T00_001", short]);
        let logos = format!("T00_001.{short}");
        let (imported, doc) = emit_retaining(T00_001, &target);
        let fresh = reparse(&doc);
        assert_matches_import(&imported, &doc, fresh.context());
        let reference = reference::reference_declaration(T00_001, &target).expect("reference");
        emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{short}: {f}"));
        assert_eq!(
            telescope_names(&body(fresh.context(), &logos).unwrap()),
            names,
            "{short}: its written binders"
        );
        let prefix = format!("`{logos}`: ");
        let recorded: Vec<&str> = doc
            .losses
            .iter()
            .filter(|(c, _)| c == "binder_renamed")
            .filter_map(|(_, d)| d.strip_prefix(prefix.as_str()))
            .collect();
        assert_eq!(recorded, renames, "{short}: its recorded renames");
        assert_binder_names_are_honest(&doc, fresh.context());
    }

    // The silent mistranslation the case guards against: every binder written under its Lean hint `x`,
    // so the body names the INNERMOST binder. It is a different declaration, refused by name; the same
    // declaration under other honest names is accepted.
    let target = key(&["T00_001", "shadowedOutermost"]);
    let (_, doc) = emit_retaining(T00_001, &target);
    let (first, last) = entry(&doc, &target).lines.expect("written");
    let replaced = |definition: &str| {
        let mut lines: Vec<&str> = doc.source.lines().collect();
        lines.splice(first - 1..last, [definition]);
        let mut forged = doc.clone();
        forged.source = lines.join("\n") + "\n";
        forged
    };
    let honest = "Definition T00_001.shadowedOutermost : Nat -> Nat -> Nat -> Nat := fun (x : Nat) (y : Nat) (z : Nat) => x.";
    emit::verify(&replaced(honest)).expect("the same declaration under other binder names");
    let captured = "Definition T00_001.shadowedOutermost : Nat -> Nat -> Nat -> Nat := fun (x : Nat) (x : Nat) (x : Nat) => x.";
    assert_eq!(
        emit::verify(&replaced(captured)).err(),
        Some(EmissionFault::Body {
            logos: "T00_001.shadowedOutermost".into()
        })
    );

    // T06-002: `transport` writes `a` as `a`; the only rename in its document is the motive's hygienic `h✝`
    // under the proof's own `h`. `Eq`'s index binder is an arrow, and no rename is claimed for it.
    let (_, doc) = emit_retaining(T06_002, &key(&["T06_002", "transport"]));
    let fresh = reparse(&doc);
    assert_binder_names_are_honest(&doc, fresh.context());
    let renamed: Vec<&str> = doc
        .losses
        .iter()
        .filter(|(c, _)| c == "binder_renamed")
        .map(|(_, d)| d.as_str())
        .collect();
    assert_eq!(
        renamed,
        [
            "`Lean.Eq.rec`: the Lean binder `a` is written `a1` because an enclosing binder already has that name",
            "`T06_002.transport`: the Lean binder `h` is written `h1` because an enclosing binder already has that name",
        ]
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Name collision
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A Lean name's components, reading `«…»` as one component.
fn lean_components(display: &str) -> Vec<String> {
    let (mut parts, mut current, mut quoted) = (Vec::new(), String::new(), false);
    for ch in display.chars() {
        match ch {
            '\u{ab}' => quoted = true,
            '\u{bb}' => quoted = false,
            '.' if !quoted => parts.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    parts.push(current);
    parts
}

#[test]
fn a_lean_name_the_prelude_already_uses_is_relocated_whole_and_never_merged() {
    let target = key(&["collisions"]);
    let (imported, doc) = emit_retaining(LE_EMIT, &target);
    let prelude = prelude_keys();
    let roots: BTreeSet<String> = prelude
        .iter()
        .map(|k| k.split('.').next().unwrap().to_string())
        .collect();
    let lean_roots: BTreeSet<String> = doc
        .correspondence
        .iter()
        .map(|c| lean_components(&c.lean)[0].clone())
        .collect();
    let relocation = std::iter::successors(Some("Lean".to_string()), |s| Some(format!("{s}'")))
        .find(|c| !roots.contains(c) && !lean_roots.contains(c))
        .unwrap();
    let placed = |lean_root: &str, rest: &str| {
        if roots.contains(lean_root) {
            format!("{relocation}.{rest}")
        } else {
            rest.to_string()
        }
    };

    assert!(
        prelude.contains("add"),
        "the fixture relies on the prelude declaring `add`"
    );
    assert_eq!(
        entry(&doc, &key(&["add"])).logos,
        format!("{relocation}.add")
    );
    assert_eq!(entry(&doc, &key(&["a_b"])).logos, placed("a_b", "a_b"));
    assert_eq!(
        entry(&doc, &key(&["a b"])).logos,
        placed("a_b", "a_b'"),
        "a sanitized name steps aside"
    );
    assert_eq!(entry(&doc, &key(&["x", "y"])).logos, placed("x", "x.y"));
    assert_eq!(
        entry(&doc, &key(&["x.y"])).logos,
        placed("x_y", "x_y"),
        "one component stays one component"
    );
    assert_eq!(
        entry(&doc, &key(&["collisions"])).logos,
        placed("collisions", "collisions")
    );
    let nat = entry(&doc, "Nat");
    assert_eq!(
        (nat.logos.as_str(), &nat.origin, nat.form, nat.lines),
        ("Nat", &Origin::Prelude, Form::Prelude, None)
    );
    assert_eq!(entry(&doc, "Zero").logos, "Zero");
    assert_eq!(entry(&doc, "Succ").logos, "Succ");
    assert_eq!(
        entry(&doc, "Nat/s:\"add\"").logos,
        format!("{relocation}.Nat.add"),
        "Lean's Nat namespace is not the prelude's"
    );

    // Injective, never a prelude key, and namespace structure preserved: two entries share a LOGOS
    // namespace exactly when they share a Lean namespace.
    let mut seen = BTreeMap::new();
    for c in &doc.correspondence {
        assert!(
            seen.insert(c.logos.clone(), c.lean.clone()).is_none(),
            "{} named twice",
            c.logos
        );
        if c.origin != Origin::Prelude {
            assert!(
                !prelude.contains(&c.logos),
                "{} collides with the prelude",
                c.logos
            );
        }
    }
    // Lean's names only: the reader's own constants (a structure's projections, sharing helpers) carry no
    // Lean name and live inside the declaration they serve.
    let written: Vec<&Correspondence> = doc
        .correspondence
        .iter()
        .filter(|c| {
            c.origin != Origin::Prelude && !c.importer_key.is_empty() && c.lean != c.importer_key
        })
        .collect();
    for a in &written {
        for b in &written {
            let lean_ns = |c: &Correspondence| {
                let p = lean_components(&c.lean);
                p[..p.len() - 1].to_vec()
            };
            let logos_ns = |c: &Correspondence| {
                let p: Vec<&str> = c.logos.split('.').collect();
                p[..p.len() - 1].join(".")
            };
            if logos_ns(a) == logos_ns(b) {
                assert_eq!(
                    lean_ns(a),
                    lean_ns(b),
                    "{} and {} share a LOGOS namespace, so they must share a Lean one ({} / {})",
                    a.logos,
                    b.logos,
                    a.lean,
                    b.lean
                );
            }
            if lean_ns(a) == lean_ns(b) && !lean_ns(a).is_empty() {
                assert_eq!(
                    logos_ns(a),
                    logos_ns(b),
                    "the Lean namespace of {} and {} is one LOGOS namespace",
                    a.lean,
                    b.lean
                );
            }
        }
    }
    assert_matches_import(&imported, &doc, reparse(&doc).context());
}

/// A Lean root that merely SHARES its name with a prelude identity is not that identity: Lean's class
/// `Zero` is not the prelude's `Zero` (Lean's `Nat.zero`), so it is relocated like any other Lean name
/// the prelude already uses, and the two are never merged.
#[test]
fn a_lean_root_named_like_a_prelude_identity_is_relocated_not_merged() {
    // `LE_Names.viaZero : Nat := Zero.zero` rests on Lean's `Zero` class (corpus cases T10-009 and T10-013 do too,
    // behind exports whose import alone outlasts a test).
    let target = key(&["LE_Names", "viaZero"]);
    let (imported, doc) = emit_retaining(LE_NAMES, &target);
    let zero_class = entry(&doc, &key(&["Zero"]));
    assert_eq!(
        (zero_class.logos.as_str(), zero_class.lean.as_str()),
        ("Lean.Zero", "Zero")
    );
    assert_eq!(
        entry(&doc, &key(&["Zero", "mk"])).logos,
        "Lean.Zero.mk",
        "its namespace moves with it"
    );
    let fresh = reparse(&doc);
    assert!(fresh.context().is_inductive("Lean.Zero") && fresh.context().is_constructor("Zero"));
    assert_matches_import(&imported, &doc, fresh.context());
    let reference = reference::reference_declaration(LE_NAMES, &target).expect("reference");
    emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{f}"));

    // The same, by hand: `C.z : Zero := Nat.zero`, where `Zero` is a Lean root (an alias of Nat) and the
    // value is the prelude's `Zero`.
    let mut ctx = nat_context();
    let alias = key(&["Zero"]);
    ctx.declare(&alias, Term::Sort(Universe::Type(0)), nat())
        .expect("a Lean root named Zero");
    let z = key(&["C", "z"]);
    ctx.declare(&z, Term::Global(alias.clone()), Term::Global("Zero".into()))
        .expect("C.z");
    let doc = emit::emit_context(&ctx, &|_| None, &z).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(entry(&doc, &alias).logos, "Lean.Zero");
    let fresh = reparse(&doc);
    assert_matches_import(&ctx, &doc, fresh.context());
    assert_eq!(
        header(fresh.context(), "C.z").map(|(_, ty)| ty),
        Some(Term::Global("Lean.Zero".into()))
    );
    assert_eq!(
        body(fresh.context(), "C.z"),
        Some(Term::Global("Zero".into())),
        "the prelude's own Zero"
    );
    emit::verify(&doc).expect("verified");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Universe preservation
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn universe_parameters_and_levels_are_preserved_exactly() {
    let u = || Universe::Var("u".into());
    let v = || Universe::Var("v".into());
    for (short, levels) in [
        ("pick", vec!["u", "v"]),
        ("Arrow", vec!["u", "v"]),
        ("Lift", vec!["u", "v"]),
        ("pick_self", vec!["u"]),
    ] {
        let target = le(short);
        let (imported, doc) = emit_retaining(LE_EMIT, &target);
        let c = entry(&doc, &target);
        let logos_levels: Vec<&str> = c.levels.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            logos_levels, levels,
            "{short}: LOGOS universe parameters, in Lean's order"
        );
        let fresh = reparse(&doc);
        let (fresh_levels, _) = header(fresh.context(), &c.logos).unwrap();
        assert_eq!(fresh_levels, levels, "{short}: the reparsed telescope");
        assert_matches_import(&imported, &doc, fresh.context());
    }
    // `Arrow (α : Sort u) (β : Sort v) : Sort (imax u v)` — the impredicative maximum survives as
    // itself, not as `max`.
    let (_, doc) = emit_retaining(LE_EMIT, &le("Arrow"));
    let fresh = reparse(&doc);
    let (_, ty) = header(fresh.context(), "LE_Emit.Arrow").unwrap();
    let mut t = &ty;
    while let Term::Pi { body_type, .. } = t {
        t = body_type;
    }
    assert_eq!(t, &Term::Sort(Universe::IMax(Box::new(u()), Box::new(v()))));
}

#[test]
fn a_changed_universe_telescope_is_refused() {
    let target = le("pick");
    let (_, doc) = emit_retaining(LE_EMIT, &target);
    let (_, twin) = emit_retaining(TWIN, &target);
    let mut forged = doc.clone();
    forged.source = twin.source.clone();
    assert!(
        matches!(emit::verify(&forged), Err(EmissionFault::Levels { ref logos, .. }) if logos == "LE_Emit.pick"),
        "{:?}",
        emit::verify(&forged).err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Helper provenance
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A kernel export whose definition `wide` repeats one closed `let` value `rounds` times — the
/// shape the reader keeps compact by introducing checked sharing helpers (the arena's own
/// `lean_arena_wide_let_sharing` input, under the Stage-A header the intake requires).
fn wide_let_export(rounds: usize) -> String {
    use serde_json::{json, Value};
    let mut rows = vec![
        json!({"meta":{
            "lean":{"version":"4.14.0","githash":"410fab7284703f41660ca2454218dcca9b2ec896"},
            "format":{"version":"3.1.0"},
            "exporter":{"version":"3.1.0","name":"lean4export"}}}),
        json!({"in":1,"str":{"pre":0,"str":"choose"}}),
        json!({"in":2,"str":{"pre":0,"str":"wide"}}),
        json!({"il":1,"succ":0}),
    ];
    fn expr(rows: &mut Vec<Value>, next: &mut usize, key: &str, value: Value) -> usize {
        let id = *next;
        *next += 1;
        rows.push(json!({"ie":id,key:value}));
        id
    }
    let mut next = 0;
    let ty = expr(&mut rows, &mut next, "sort", json!(1));
    let leaf = expr(&mut rows, &mut next, "sort", json!(0));
    let b0 = expr(&mut rows, &mut next, "bvar", json!(0));
    let b1 = expr(&mut rows, &mut next, "bvar", json!(1));
    let inner = expr(
        &mut rows,
        &mut next,
        "lam",
        json!({"name":0,"type":ty,"body":b1,"binderInfo":"default"}),
    );
    let body = expr(
        &mut rows,
        &mut next,
        "lam",
        json!({"name":0,"type":ty,"body":inner,"binderInfo":"default"}),
    );
    let inner_ty = expr(
        &mut rows,
        &mut next,
        "forallE",
        json!({"name":0,"type":ty,"body":ty,"binderInfo":"default"}),
    );
    let fun_ty = expr(
        &mut rows,
        &mut next,
        "forallE",
        json!({"name":0,"type":ty,"body":inner_ty,"binderInfo":"default"}),
    );
    rows.push(json!({"def":{"name":1,"all":[1],"type":fun_ty,"value":body,"levelParams":[],"safety":"safe","hints":"abbrev"}}));
    let choose = expr(&mut rows, &mut next, "const", json!({"name":1,"us":[]}));
    let partial = expr(&mut rows, &mut next, "app", json!({"fn":choose,"arg":b0}));
    let value = expr(&mut rows, &mut next, "app", json!({"fn":partial,"arg":b0}));
    let mut body = b0;
    for _ in 0..rounds {
        body = expr(
            &mut rows,
            &mut next,
            "letE",
            json!({"name":0,"type":ty,"value":value,"body":body,"nondep":false}),
        );
    }
    body = expr(
        &mut rows,
        &mut next,
        "letE",
        json!({"name":0,"type":ty,"value":leaf,"body":body,"nondep":false}),
    );
    rows.push(json!({"def":{"name":2,"all":[2],"type":ty,"value":body,"levelParams":[],"safety":"safe","hints":"abbrev"}}));
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn a_reader_helper_is_written_as_a_checked_definition_marked_with_its_origin() {
    let export = wide_let_export(160);
    let target = key(&["wide"]);
    let intake = adapter::intake::read(&export, &adapter::intake::STAGE_A).expect("intake");
    let closure = intake.closure(&target).expect("closure");
    let (imported, report) =
        adapter::import_retaining_closure(&export, &target, &closure).expect("imports");
    let doc = emit_from(&imported, &target);
    let reported: BTreeSet<&str> = report
        .internal_helpers
        .iter()
        .map(|h| h.name.as_str())
        .collect();
    let helpers: Vec<&Correspondence> = doc
        .correspondence
        .iter()
        .filter(|c| c.importer_key.starts_with("arena.internal.dag."))
        .collect();
    assert!(
        !helpers.is_empty(),
        "the reader kept `wide` compact through sharing helpers"
    );
    assert_eq!(
        helpers
            .iter()
            .map(|h| h.importer_key.as_str())
            .collect::<BTreeSet<_>>(),
        reported,
        "every helper the import reported is written, and nothing else is called one"
    );
    let mut logos: Vec<&str> = helpers.iter().map(|h| h.logos.as_str()).collect();
    logos.sort();
    let mut numbered: Vec<String> = (1..=helpers.len())
        .map(|i| format!("wide.shared_{i}"))
        .collect();
    numbered.sort();
    assert_eq!(
        logos, numbered,
        "helpers are named inside the declaration whose sharing they keep"
    );
    for h in &helpers {
        assert_eq!(
            h.origin,
            Origin::Helper { of: "wide".into() },
            "{}",
            h.importer_key
        );
        assert_eq!(
            h.lean, h.importer_key,
            "a helper has no Lean name and does not pretend to"
        );
        assert_eq!(h.kind, DeclarationKind::Definition);
        assert_ne!(h.form, Form::Prelude, "a helper is written");
        let text = block(&doc, h);
        let (first, _) = h.lines.unwrap();
        let prose = doc.source.lines().nth(first - 2).unwrap();
        assert!(
            prose.contains(&h.importer_key) && prose.contains("`wide`"),
            "{prose}\n{text}"
        );
    }
    for c in &doc.correspondence {
        if !c.importer_key.starts_with("arena.internal.dag.") {
            assert!(
                !matches!(c.origin, Origin::Helper { .. }),
                "{} is not a helper",
                c.logos
            );
        }
    }
    assert_eq!(entry(&doc, &target).origin, Origin::Authored);
    assert_matches_import(&imported, &doc, reparse(&doc).context());
}

#[test]
fn a_derived_recursor_is_marked_synthesized() {
    let (_, doc) = emit_retaining(LE_EMIT, &le("double"));
    let rec = entry(&doc, "Nat/s:\"rec\"");
    assert_eq!(
        (rec.kind, &rec.origin),
        (DeclarationKind::Definition, &Origin::Synthesized)
    );
    assert_eq!(rec.lean, "Nat.rec");
    for c in &doc.correspondence {
        let recursor = c.importer_key.ends_with("/s:\"rec\"");
        let derived_here = c.importer_key.is_empty();
        let projection = c.importer_key.starts_with("arena.structure.projection.");
        assert_eq!(
            c.origin == Origin::Synthesized,
            recursor || derived_here || projection,
            "{}",
            c.logos
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Altered target type, and every other single-respect corruption
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The document for `target` from the original export, with its source replaced by the twin's.
fn forged(target: &str, plain: bool) -> (EmittedDocument, EmittedDocument) {
    let (_, doc) = if plain {
        emit_plain(LE_EMIT, target)
    } else {
        emit_retaining(LE_EMIT, target)
    };
    let (_, twin) = if plain {
        emit_plain(TWIN, target)
    } else {
        emit_retaining(TWIN, target)
    };
    let mut forged = doc;
    forged.source = twin.source.clone();
    (forged, twin)
}

#[test]
fn an_altered_target_type_is_refused() {
    let (forged, twin) = forged(&le("double_two"), false);
    assert_eq!(
        emit::verify(&forged).err(),
        Some(EmissionFault::Type {
            logos: "LE_Emit.double_two".into()
        })
    );
    // The twin's own document is honest about ITS statement, so it verifies — and the reference
    // translation of the ORIGINAL Lean statement refuses it.
    emit::verify(&twin).expect("the twin's document is its own honest emission");
    let original = reference::reference_declaration(LE_EMIT, &le("double_two")).expect("reference");
    assert_eq!(
        emit::check_statement(&twin, &original).err(),
        Some(EmissionFault::Statement {
            logos: "LE_Emit.double_two".into()
        })
    );
    let dir = scratch_dir("altered");
    assert!(matches!(
        emit::publish(&dir, "LE_Emit.double_two", &Ok(forged)).expect("io"),
        Published::Failure(_)
    ));
    assert!(
        !dir.join("LE_Emit.double_two.lg").exists(),
        "an altered document is never published"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_single_respect_in_which_the_twin_differs_is_named() {
    let (f, _) = forged(&le("two"), false);
    assert_eq!(
        emit::verify(&f).err(),
        Some(EmissionFault::Body {
            logos: "LE_Emit.two".into()
        })
    );
    let (f, _) = forged(&le("favorite"), false);
    assert_eq!(
        emit::verify(&f).err(),
        Some(EmissionFault::Constructors {
            logos: "LE_Emit.Color".into()
        })
    );
    let (f, _) = forged(&le("Oracle"), true);
    assert_eq!(
        emit::verify(&f).err(),
        Some(EmissionFault::Type {
            logos: "LE_Emit.Oracle".into()
        })
    );
    let (f, _) = forged(&le("uses_oracle"), false);
    assert!(
        matches!(emit::verify(&f), Err(EmissionFault::Kind { ref logos, expected: DeclarationKind::Theorem }) if logos == "LE_Emit.uses_oracle")
            || matches!(emit::verify(&f), Err(EmissionFault::Type { ref logos }) if logos == "LE_Emit.Oracle"),
        "a transparent definition is not the imported theorem: {:?}",
        emit::verify(&f).err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Missing dependency
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_dependency_the_environment_lacks_is_refused_by_name() {
    let mut ctx = nat_context();
    let absent = key(&["C", "absent"]);
    let dangling = key(&["C", "dangling"]);
    ctx.add_declaration(&dangling, Term::Global(absent.clone()));
    let (code, message, rejected) = decline_code(emit::emit_context(&ctx, &|_| None, &dangling));
    assert!(
        rejected,
        "a dangling reference is a defect of the input: {message}"
    );
    assert_eq!(code, DiagnosticCode::MissingDeclaration);
    assert!(
        message.contains("C.absent") && message.contains("C.dangling"),
        "{message}"
    );

    let (code, message, rejected) =
        decline_code(emit::emit_context(&ctx, &|_| None, &key(&["C", "nowhere"])));
    assert!(rejected, "{message}");
    assert_eq!(code, DiagnosticCode::MissingDeclaration, "{message}");
}

#[test]
fn a_document_missing_a_dependency_does_not_reparse() {
    let target = le("area");
    let (_, doc) = emit_retaining(LE_EMIT, &target);
    let shape = entry(&doc, &le("Shape")).clone();
    let mut missing = doc.clone();
    missing.source = without(&doc, &shape);
    assert!(
        matches!(emit::verify(&missing), Err(EmissionFault::Reparse { .. })),
        "{:?}",
        emit::verify(&missing).err()
    );
    let mut fresh = Repl::new();
    assert!(
        fresh.execute_document(&missing.source).is_err(),
        "a fresh toolchain refuses it too"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Deterministic emission
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn emission_is_deterministic() {
    let wide: &'static str = Box::leak(wide_let_export(160).into_boxed_str());
    for (export, target) in [
        (LE_EMIT, key(&["collisions"])),
        (LE_EMIT, le("area")),
        (LE_EMIT, le("pick_self")),
        (LE_EMIT, le("double_two")),
        (wide, key(&["wide"])),
    ] {
        let (_, first) = emit_retaining(export, &target);
        for _ in 0..2 {
            let (_, again) = emit_retaining(export, &target);
            assert_eq!(
                first, again,
                "{target}: independent imports emit byte-identical documents"
            );
        }
        let from_thread = std::thread::spawn(move || emit_retaining(export, &target).1)
            .join()
            .unwrap();
        assert_eq!(first, from_thread);
    }
}

#[test]
fn the_document_depends_on_the_checked_declarations_alone() {
    use logicaffeine_kernel::elaborate::ParamKind;
    for target in [le("pick_self"), le("area"), key(&["T06_002", "transport"])] {
        let export = if target.contains("T06_002") {
            T06_002
        } else {
            LE_EMIT
        };
        let (imported, doc) = emit_retaining(export, &target);
        // The same checked declarations with every binder annotation erased — what a replay rebuilds
        // from a certificate — write the identical document and correspondence.
        let mut bare: Context = (*imported).clone();
        for c in doc
            .correspondence
            .iter()
            .filter(|c| !c.importer_key.is_empty())
        {
            if let Some(kinds) = imported.binder_kinds(&c.importer_key) {
                bare.set_binder_kinds(&c.importer_key, vec![ParamKind::Explicit; kinds.len()]);
            }
        }
        let proofs = |k: &str| imported.retained(k).map(|(_, body)| body.clone());
        let replayed = emit::emit_context(&bare, &proofs, &target)
            .unwrap_or_else(|e| panic!("{target}: {e:?}"));
        assert_eq!(
            replayed.source, doc.source,
            "{target}: the bytes do not depend on import metadata"
        );
        assert_eq!(replayed.correspondence, doc.correspondence, "{target}");
        assert!(
            replayed
                .losses
                .iter()
                .all(|(category, _)| category != "binder_annotations"),
            "{target}: nothing annotated, no annotation lost"
        );
        let renames = |d: &EmittedDocument| {
            d.losses
                .iter()
                .filter(|(c, _)| c == "binder_renamed")
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            renames(&replayed),
            renames(&doc),
            "{target}: binder renames are read off the terms alone"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Partial-write failure
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lean-emission-{tag}-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn listing(dir: &std::path::Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect()
}

#[test]
fn a_failed_or_partial_write_never_leaves_a_success() {
    let (_, doc) = emit_retaining(LE_EMIT, &le("two"));
    let dir = scratch_dir("publish");
    let stem = "LE_Emit.two";

    // A clean run publishes exactly the document.
    let published = emit::publish(&dir, stem, &Ok(doc.clone())).expect("io");
    assert_eq!(published, Published::Document(dir.join("LE_Emit.two.lg")));
    assert_eq!(
        std::fs::read_to_string(dir.join("LE_Emit.two.lg")).unwrap(),
        doc.source
    );
    assert_eq!(
        listing(&dir),
        BTreeSet::from(["LE_Emit.two.lg".to_string()])
    );

    // A write that dies half way leaves neither a partial file nor the earlier success.
    let mut half = |sink: &mut dyn std::io::Write, bytes: &[u8]| -> std::io::Result<()> {
        sink.write_all(&bytes[..bytes.len() / 2])?;
        Err(std::io::Error::new(std::io::ErrorKind::Other, "disk full"))
    };
    let failed = emit::publish_through(&dir, stem, &Ok(doc.clone()), &mut half);
    assert!(failed.is_err(), "the failed write is reported");
    assert_eq!(
        listing(&dir),
        BTreeSet::new(),
        "no staging file, no partial file, no stale success"
    );

    // A failed emission publishes no document, removes the earlier success, and records why.
    emit::publish(&dir, stem, &Ok(doc.clone())).expect("io");
    // The emission refuses a genuinely opaque declaration whose checked value was not retained.
    // (A theorem is published unfoldably, as in Lean, so its proof is always in the context.)
    let plain = import_plain(LE_EMIT);
    let mut sealed: logicaffeine_kernel::Context = (*plain).clone();
    let theorem = le("double_two");
    let sealed_key = le("sealed_double_two");
    sealed
        .declare_opaque_checked(
            &sealed_key,
            vec![],
            plain.get_global(&theorem).unwrap().clone(),
            plain.get_definition_body(&theorem).unwrap().clone(),
        )
        .unwrap();
    let refusal = emit::emit_context(&sealed, &|_| None, &sealed_key);
    assert!(refusal.is_err());
    let published = emit::publish(&dir, stem, &refusal).expect("io");
    assert_eq!(
        published,
        Published::Failure(dir.join("LE_Emit.two.failure.txt"))
    );
    assert_eq!(
        listing(&dir),
        BTreeSet::from(["LE_Emit.two.failure.txt".to_string()])
    );
    let why = std::fs::read_to_string(dir.join("LE_Emit.two.failure.txt")).unwrap();
    assert!(why.starts_with("decline unsupported_feature: "), "{why}");

    // A later success replaces the failure record.
    emit::publish(&dir, stem, &Ok(doc.clone())).expect("io");
    assert_eq!(
        listing(&dir),
        BTreeSet::from(["LE_Emit.two.lg".to_string()])
    );

    // A document altered after emission is re-verified from its bytes and never published.
    let mut tampered = doc.clone();
    tampered.source = without(&doc, entry(&doc, &le("two")));
    assert!(matches!(
        emit::publish(&dir, stem, &Ok(tampered)).expect("io"),
        Published::Failure(_)
    ));
    assert!(!dir.join("LE_Emit.two.lg").exists());

    for bad in ["", ".", "..", "a/b", ".hidden"] {
        assert!(
            emit::publish(&dir, bad, &Ok(doc.clone())).is_err(),
            "{bad:?} is not a file stem"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rule R8
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_emitted_statement_matches_the_reference_translation() {
    let cases: [(&str, String, bool); 8] = [
        (LE_EMIT, le("double_two"), false),
        (LE_EMIT, le("area"), false),
        (LE_EMIT, le("pick"), false),
        (LE_EMIT, le("pick_self"), false),
        (LE_EMIT, le("Oracle"), true),
        (T06_002, key(&["T06_002", "transport"]), false),
        (T06_002, key(&["T06_002", "jDependent"]), false),
        (LE_EMIT, key(&["LE_Emit", "Shape", "box"]), true),
    ];
    for (export, target, plain) in &cases {
        let (_, doc) = if *plain {
            emit_plain(export, target)
        } else {
            emit_retaining(export, target)
        };
        assert_eq!(
            doc.target,
            entry(&doc, target).logos,
            "the document's target is the requested declaration"
        );
        let reference =
            reference::reference_declaration(export, target).expect("reference translation");
        emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{target}: {f}"));
    }

    // The comparison is not vacuous: another declaration's reference, the twin's statement of the
    // same name, and the twin's universe order are each refused.
    let (_, doc) = emit_retaining(LE_EMIT, &le("double_two"));
    let other = reference::reference_declaration(LE_EMIT, &le("pick_self")).unwrap();
    assert_eq!(
        emit::check_statement(&doc, &other).err(),
        Some(EmissionFault::Statement {
            logos: "LE_Emit.double_two".into()
        })
    );
    let twin = reference::reference_declaration(TWIN, &le("double_two")).unwrap();
    assert_eq!(
        emit::check_statement(&doc, &twin).err(),
        Some(EmissionFault::Statement {
            logos: "LE_Emit.double_two".into()
        })
    );
    let (_, pick) = emit_retaining(LE_EMIT, &le("pick"));
    let twin_pick = reference::reference_declaration(TWIN, &le("pick")).unwrap();
    assert!(
        emit::check_statement(&pick, &twin_pick).is_err(),
        "a reordered universe telescope is a different statement"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Every original fixture
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The original fixtures no emission test holds, each with the reason. Nothing else may be missing.
const NOT_EMITTED: &[(&str, &str)] = &[(
    "T00-017",
    "its 3.5 MB export alone takes longer to import than a test may run (`vernacular_roundtrip::fixture_t00_017_round_trips` \
     times out on the import; lane LG, W06)",
)];

fn original_fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414")
}

/// The Lean declarations an original fixture names: its provenance's `declarations`, and for T00-001,
/// whose provenance records none, the seven its source (`T00-001_de_bruijn_to_named_var.lean`) declares.
fn original_declarations(stem: &str) -> Vec<String> {
    if stem == "T00-001" {
        return [
            "identityNat",
            "pickOuter",
            "shadowedInnermost",
            "shadowedOutermost",
            "usesSectionVar",
            "binderNamedLikeAGlobal",
            "innerTypeMentionsOuter",
        ]
        .iter()
        .map(|d| format!("T00_001.{d}"))
        .collect();
    }
    let path = original_fixtures_dir().join(format!("{stem}.provenance.json"));
    let provenance: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .expect("provenance JSON");
    let declarations: Vec<String> = provenance["declarations"]
        .as_array()
        .unwrap_or_else(|| panic!("{stem}: its provenance names its declarations"))
        .iter()
        .map(|d| d.as_str().expect("a declaration name").to_string())
        .collect();
    assert!(
        !declarations.is_empty(),
        "{stem}: its provenance names its declarations"
    );
    declarations
}

/// Emit every one of `declarations` from ONE import of `export` that retains all their closures, and
/// hold each document to the import, to the reference translation of its Lean statement (R8), and to
/// the truth about its binder names.
fn assert_every_declaration_emits(export: &str, declarations: &[String]) {
    let intake = adapter::intake::read(export, &adapter::intake::STAGE_A)
        .unwrap_or_else(|e| panic!("intake: {e:?}"));
    let keys: Vec<String> = declarations
        .iter()
        .map(|d| key(&d.split('.').collect::<Vec<_>>()))
        .collect();
    let mut closure: Vec<String> = Vec::new();
    for k in &keys {
        closure.extend(
            intake
                .closure(k)
                .unwrap_or_else(|e| panic!("closure of {k}: {e:?}")),
        );
    }
    closure.sort();
    closure.dedup();
    let imported = adapter::import_audited_retaining(export, &closure)
        .unwrap_or_else(|e| panic!("import: {e:?}"))
        .0;
    for (lean, k) in declarations.iter().zip(&keys) {
        let doc = emit_from(&imported, k);
        assert_eq!(entry(&doc, k).lean, *lean, "the document carries {lean}");
        let fresh = reparse(&doc);
        assert_matches_import(&imported, &doc, fresh.context());
        let reference = reference::reference_declaration(export, k)
            .unwrap_or_else(|e| panic!("{lean}: reference: {e:?}"));
        emit::check_statement(&doc, &reference).unwrap_or_else(|f| panic!("{lean}: {f}"));
        assert_binder_names_are_honest(&doc, fresh.context());
    }
}

macro_rules! original_emission_tests {
    ($($test:ident: $stem:literal),* $(,)?) => {
        /// Every original fixture an emission test holds.
        const EMITTED_ORIGINALS: &[&str] = &[$($stem),*];

        /// One test per original fixture: every declaration it names is emitted, reparsed, compared
        /// with the import, bound to its Lean statement, and honest about its binder names.
        mod every_original_fixture_emits {
            use super::*;
            $(
                #[test]
                fn $test() {
                    let export = include_str!(concat!("fixtures/lean_conformance/cases_lean414/", $stem, ".ndjson"));
                    assert_every_declaration_emits(export, &original_declarations($stem));
                }
            )*
        }
    };
}

original_emission_tests! {
    t00_001: "T00-001",
    t00_002: "T00-002",
    t00_003_structural: "T00-003.structural",
    t00_004: "T00-004",
    t00_005: "T00-005",
    t00_005_structural: "T00-005.structural",
    t00_014: "T00-014",
    t00_019_sharing_structural: "T00-019.sharing.structural",
    t02_014_forest_size: "T02-014.forestSize",
    t03_008_binduction_on_1: "T03-008.binductionOn_1",
    t04_015_acc_depth: "T04-015.accDepth",
    t04_015_four: "T04-015.four",
    t06_002: "T06-002",
    t06_006_exists_nested: "T06-006.existsNested",
}

#[test]
fn every_original_fixture_is_held_to_its_emission() {
    let on_disk: BTreeSet<String> = std::fs::read_dir(original_fixtures_dir())
        .expect("the original fixtures")
        .filter_map(|e| {
            e.ok()?
                .file_name()
                .to_str()?
                .strip_suffix(".ndjson")
                .map(str::to_string)
        })
        .collect();
    let held: BTreeSet<String> = EMITTED_ORIGINALS
        .iter()
        .chain(NOT_EMITTED.iter().map(|(stem, _)| stem))
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        on_disk, held,
        "every original fixture is emitted by a test, or excluded with its reason"
    );
    for (stem, why) in NOT_EMITTED {
        assert!(
            !EMITTED_ORIGINALS.contains(stem) && !why.is_empty(),
            "{stem}"
        );
    }
}

/// W05, lane LD's proof-package fixtures: each target is its own export.
#[test]
fn every_w05_package_target_emits() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lean_package");
    let provenance: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("W05.provenance.json")).unwrap())
            .unwrap();
    let exports = provenance["exports"].as_object().expect("W05 exports");
    assert_eq!(
        exports.len(),
        7,
        "clean, tainted, byGoalAxiom, usesPropext, usesFunext, usesChoice, restatesPropext"
    );
    for record in exports.values() {
        let export = std::fs::read_to_string(dir.join(record["file"].as_str().unwrap())).unwrap();
        assert_every_declaration_emits(
            &export,
            &[record["declaration"].as_str().unwrap().to_string()],
        );
    }
}

/// LE's own naming fixture, declaration by declaration, from its provenance.
#[test]
fn every_le_names_declaration_emits() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lean_emission");
    let provenance: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("PROVENANCE.json")).unwrap())
            .unwrap();
    let fixture = provenance["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["export"] == "LE_Names.ndjson")
        .expect("LE_Names is recorded");
    let declarations: Vec<String> = fixture["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap().to_string())
        .collect();
    assert_eq!(declarations.len(), 7);
    assert_every_declaration_emits(LE_NAMES, &declarations);
}
