//! The bridge-harvest checker: one original Lean declaration, from its pinned export, to a checked
//! LOGOS artifact bound to the independent reference translation of its Lean type.
//!
//! For each root:
//! * the fixture is the pinned original export (sha256, header, toolchain, exporter and the
//!   unchanged original source all match its provenance);
//! * the production reader imports it once (`import_audited_retaining`), which checks every declaration body — a
//!   theorem's opaque proof included — in the primary kernel and in its independent re-check;
//! * the declaration's statement is exactly convertible with the reference translator's reading of the
//!   Lean type — the translator shares no code with the importer;
//! * the primary kernel AND the independent re-checker each check the declaration against that
//!   reference statement (a definition's body, a theorem's retained opaque proof, and the declared constant);
//! * the LOGOS axiom audit equals Lean's recorded `#print axioms`, name for name;
//! * per group, a corruption control: the same constant offered at an altered statement is refused by
//!   both checkers, after the unaltered statement was accepted by both.

#![allow(dead_code)]

use adapter::reference::{
    kernel_name_of_components, Reference, ReferenceDeclaration, ReferenceKind,
};
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, infer_type, recheck, BigInt, Context, Literal, Term,
    Universe,
};
use serde_json::Value;
use std::path::PathBuf;

/// One original root, with the facts Lean recorded for it.
pub struct Root {
    pub lean: &'static str,
    pub kind: &'static str,
    pub axioms: &'static [&'static str],
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The pinned export and its provenance, verified byte for byte against each other and against the
/// unchanged original source.
pub fn fixture(stem: &str) -> (String, Value) {
    fixture_in(
        &repository().join(
            "crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/harvest",
        ),
        stem,
    )
}

pub fn fixture_in(dir: &std::path::Path, stem: &str) -> (String, Value) {
    let export =
        std::fs::read_to_string(dir.join(format!("{stem}.ndjson"))).expect("pinned export");
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{stem}.provenance.json"))).expect("provenance"),
    )
    .expect("provenance parses");
    assert_eq!(
        adapter::package::sha256_hex(export.as_bytes()),
        provenance["export_sha256"],
        "{stem}: export bytes"
    );
    assert_eq!(
        export.lines().count() as u64,
        provenance["records"].as_u64().unwrap(),
        "{stem}: record count"
    );
    let header: Value = serde_json::from_str(export.lines().next().unwrap()).unwrap();
    assert_eq!(header, provenance["export_header"], "{stem}: export header");
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        provenance["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(provenance["expression_equality"], "structural");
    assert_eq!(
        provenance["declaration_selection"],
        "explicit_authored_roots"
    );
    assert_eq!(provenance["compiler_exit"], 0);
    assert_eq!(provenance["exporter_exit"], 0);
    let source = std::fs::read(repository().join(provenance["source"].as_str().unwrap()))
        .expect("original source");
    assert_eq!(
        adapter::package::sha256_hex(&source),
        provenance["source_sha256"],
        "{stem}: the original source changed"
    );
    (export, provenance)
}

fn kind_of(kind: &str) -> ReferenceKind {
    match kind {
        "def" => ReferenceKind::Definition,
        "theorem" => ReferenceKind::Theorem,
        "opaque" => ReferenceKind::Opaque,
        "axiom" => ReferenceKind::Axiom,
        "inductive" => ReferenceKind::Inductive,
        "ctor" => ReferenceKind::Constructor,
        "rec" => ReferenceKind::Recursor,
        other => panic!("unknown ledger kind {other}"),
    }
}

/// The declared constant at its own universe parameters.
fn constant(declaration: &ReferenceDeclaration) -> Term {
    if declaration.level_params.is_empty() {
        Term::Global(declaration.name.clone())
    } else {
        Term::Const {
            name: declaration.name.clone(),
            levels: declaration
                .level_params
                .iter()
                .map(|p| Universe::Var(p.clone()))
                .collect(),
        }
    }
}

/// `(λ(__statement : expected). 0) term`: each checker performs its own conversion of `term`'s type to
/// `expected`, instead of two inferred types being compared by one checker.
fn against(term: &Term, expected: &Term) -> Term {
    Term::App(
        Box::new(Term::Lambda {
            param: "__statement".into(),
            param_type: Box::new(expected.clone()),
            body: Box::new(Term::Lit(Literal::Nat(BigInt::from_u64(0)))),
        }),
        Box::new(term.clone()),
    )
}

fn both_accept(ctx: &Context, term: &Term) -> Result<(), String> {
    infer_type(ctx, term).map_err(|e| format!("primary kernel refused: {e}"))?;
    recheck(ctx, term).map_err(|e| format!("independent re-checker refused: {e:?}"))?;
    Ok(())
}

fn both_refuse(ctx: &Context, term: &Term) -> bool {
    infer_type(ctx, term).is_err() && recheck(ctx, term).is_err()
}

/// What the group's corruption control needs from one checked root.
pub struct Checked {
    pub constant: Term,
    pub statement: Term,
}

/// A dotted Lean name's components, a `«…»` component kept whole.
fn lean_components(name: &str) -> Vec<String> {
    let (mut out, mut current, mut quoted) = (Vec::new(), String::new(), false);
    for ch in name.chars() {
        match ch {
            '«' if !quoted => quoted = true,
            '»' if quoted => quoted = false,
            '.' if !quoted => out.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    out.push(current);
    out
}

fn lean_axioms_in_kernel_spelling(root: &Root) -> Vec<String> {
    let mut out: Vec<String> = root
        .axioms
        .iter()
        .map(|a| {
            kernel_name_of_components(
                &lean_components(a)
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    out.sort();
    out
}

/// One import of the fixture that also retains the opaque proofs of `theorems` (kernel spellings).
fn imported(
    export: &str,
    theorems: &[String],
) -> Result<(adapter::ImportedContext, adapter::ImportReport), String> {
    adapter::import_audited_retaining(export, theorems)
        .map_err(|e| format!("production reader refused the original export: {e:?}"))
}

fn kernel_spelling(lean: &str) -> String {
    kernel_name_of_components(
        &lean_components(lean)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )
}

fn theorems(roots: &[Root]) -> Vec<String> {
    roots
        .iter()
        .filter(|r| r.kind == "theorem")
        .map(|r| kernel_spelling(r.lean))
        .collect()
}

/// Check one root in its own fresh import.
pub fn check_root(export: &str, reference: &Reference, root: &Root) -> Result<Checked, String> {
    let (ctx, report) = imported(export, &theorems(std::slice::from_ref(root)))?;
    check_in(export, &ctx, &report, reference, root)
}

/// Check one root against an import of the whole fixture. A definition's body is the one the checked
/// environment holds; a theorem's opaque proof is the one the same import retained.
pub fn check_in(
    _export: &str,
    ctx: &adapter::ImportedContext,
    report: &adapter::ImportReport,
    reference: &Reference,
    root: &Root,
) -> Result<Checked, String> {
    let components = lean_components(root.lean);
    let spelled = kernel_spelling(root.lean);
    let declaration = reference
        .declaration(&spelled)
        .map_err(|e| format!("no reference translation: {e:?}"))?;
    if declaration.lean_name != components.join(".") || declaration.kind != kind_of(root.kind) {
        return Err(format!(
            "reference reads {} as {:?}",
            declaration.lean_name, declaration.kind
        ));
    }
    let statement = declaration.ty.clone();
    let declared = ctx
        .get_global(&declaration.name)
        .cloned()
        .or_else(|| {
            ctx.get_universe_poly(&declaration.name)
                .map(|(_, ty, _)| ty.clone())
        })
        .ok_or("the checked environment does not declare it")?;
    if !defeq_for_test(ctx, &declared, &statement) {
        return Err(
            "the imported statement is not the reference translation of the Lean type".into(),
        );
    }
    let mut body_or_proof: Option<Term> = None;
    match root.kind {
        "def" => {
            let body = ctx
                .get_definition_body(&declaration.name)
                .cloned()
                .or_else(|| {
                    ctx.get_universe_poly(&declaration.name)
                        .map(|(_, _, body)| body.clone())
                })
                .ok_or("a definition without a checked body")?;
            both_accept(ctx, &against(&body, &statement))
                .map_err(|e| format!("body against the reference statement: {e}"))?;
            body_or_proof = Some(body);
        }
        "theorem" => {
            let (retained_type, proof) = ctx
                .retained(&declaration.name)
                .ok_or("the opaque proof was not retained")?;
            if !defeq_for_test(ctx, retained_type, &statement) {
                return Err("the retained proof is not stated at the reference translation".into());
            }
            both_accept(ctx, &against(proof, &statement))
                .map_err(|e| format!("proof against the reference statement: {e}"))?;
            body_or_proof = Some(proof.clone());
        }
        _ => {}
    }
    let constant = constant(&declaration);
    both_accept(ctx, &statement)
        .map_err(|e| format!("the reference statement is not a well-formed type: {e}"))?;
    both_accept(ctx, &against(&constant, &statement))
        .map_err(|e| format!("the imported constant against the reference statement: {e}"))?;

    let expected = lean_axioms_in_kernel_spelling(root);
    let observed = lean_semantics_axioms(
        ctx,
        &declaration.name,
        root.kind,
        &statement,
        body_or_proof.as_ref(),
    )?;
    if observed != expected {
        let only_lean: Vec<&String> = expected.iter().filter(|a| !observed.contains(a)).collect();
        let only_logos: Vec<&String> = observed.iter().filter(|a| !expected.contains(a)).collect();
        return Err(format!(
            "assumption_disagreement: LOGOS {observed:?} vs Lean's #print axioms {expected:?} (only in Lean: {only_lean:?}; only in LOGOS: {only_logos:?})"
        ));
    }
    Ok(Checked {
        constant,
        statement,
    })
}

/// The transitive assumptions of one declaration under Lean's `#print axioms` rule, the rule the
/// portable package applies (`package.rs`, `AssumptionRecord::transitive`): an axiom REACHED counts,
/// the axioms in an axiom's own type do not; a definition or theorem is traced through its statement
/// and its body or proof (an opaque theorem's checked evidence included); an inductive, constructor
/// or recursor through its statement. An axiom's own assumptions are the axiom itself.
pub fn lean_semantics_axioms(
    ctx: &Context,
    name: &str,
    kind: &str,
    statement: &Term,
    body_or_proof: Option<&Term>,
) -> Result<Vec<String>, String> {
    let usage = match kind {
        "axiom" => {
            let mut only = logicaffeine_kernel::AxiomUsage::default();
            only.axioms.insert(name.to_string());
            only
        }
        "def" | "theorem" | "opaque" => {
            let mut usage = axiom_dependencies(ctx, statement);
            if let Some(term) = body_or_proof {
                usage.absorb(axiom_dependencies(ctx, term));
            }
            if let Some(evidence) = logicaffeine_kernel::axiom_dependencies_of(ctx, name) {
                usage.absorb(evidence);
            }
            usage
        }
        _ => axiom_dependencies(ctx, statement),
    };
    if !usage.unresolved.is_empty() {
        return Err(format!(
            "unresolved names in the assumption trace: {:?}",
            usage.unresolved
        ));
    }
    Ok(usage.axioms.into_iter().collect())
}

/// Check every root of one named group, then the group's corruption control.
pub fn check_group(stem: &str, roots: &[Root]) {
    assert!(
        !roots.is_empty() && roots.len() <= 8,
        "a named bridge test decides one to eight declarations"
    );
    let (export, provenance) = fixture(stem);
    let pinned: Vec<&str> = provenance["harvest"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, report) =
        imported(&export, &theorems(roots)).unwrap_or_else(|e| panic!("{stem}: {e}"));
    let mut failures = Vec::new();
    let mut control = None;
    for root in roots {
        assert!(
            pinned.contains(&root.lean),
            "{}: not a root of fixture {stem}",
            root.lean
        );
        match check_in(&export, &ctx, &report, &reference, root) {
            Ok(checked) => {
                if control.is_none() {
                    control = Some(checked);
                }
            }
            Err(why) => failures.push(format!("{}: {why}", root.lean)),
        }
    }
    assert!(failures.is_empty(), "{stem}: {}", failures.join("\n"));
    let checked = control.expect("at least one checked root");
    let altered = Term::Pi {
        param: "__corrupted".into(),
        param_type: Box::new(Term::Sort(Universe::Prop)),
        body_type: Box::new(checked.statement.clone()),
    };
    assert!(
        !defeq_for_test(&ctx, &checked.statement, &altered),
        "{stem}: the conversion must discriminate"
    );
    assert!(
        both_refuse(&ctx, &against(&checked.constant, &altered)),
        "{stem}: an altered statement must be refused by both checkers"
    );
}
