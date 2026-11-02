//! W05 — the environment certificate: a proof together with every declaration it needs, rebuilt
//! and re-checked from nothing but the declared base. Deserializing is not validating: every test
//! below that tampers with a certificate asserts WHICH check refused it, so a refusal for the wrong
//! reason (a malformed row where the kernel was supposed to answer) cannot pass for the right one.
mod lean_package_support;

use lean_package_support::{capture, capture_result, declare_sorry, small_theorem, Export};
use logicaffeine_kernel::certificate::{
    check_prelude_version, read_certificate, AnyCertificate, CertDeclaration, Certificate,
    ConversionDiscipline, EnvironmentBase, EnvironmentCertificate, EnvironmentEntry,
    EnvironmentErrorCode, EnvironmentLimits, IndependentVerdict, TermNode, TheoremCertificate,
    ENVIRONMENT_SCHEMA, PRELUDE_VERSION,
};
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{defeq_for_test, Term, Universe};

const T06_002: &str = include_str!("fixtures/lean_conformance/cases_lean414/T06-002.ndjson");
const STRUCT_ETA: &str = include_str!("fixtures/lean_arena/111_structEta.ndjson");
const QUOT_LIFT: &str = include_str!("fixtures/lean_arena/129_quotLiftReduction.ndjson");
const MUTUAL: &str = include_str!("fixtures/lean_arena/ordinary-mutual-polymorphic.ndjson");
const RB_TREE: &str = include_str!("fixtures/lean_arena/045_rbTreeDef.ndjson");
const T06_ROOTS: [&str; 6] = [
    "bigMotive",
    "jDependent",
    "symmViaRec",
    "transport",
    "transportRaw",
    "viaMpr",
];

fn t06(short: &str) -> String {
    format!("/s:\"T06_002\"/s:{short:?}")
}

fn rebuild(
    certificate: &EnvironmentCertificate,
) -> Result<
    logicaffeine_kernel::certificate::RebuiltEnvironment,
    logicaffeine_kernel::certificate::EnvironmentError,
> {
    certificate.rebuild(&EnvironmentLimits::default())
}

fn refusal(certificate: &EnvironmentCertificate) -> (EnvironmentErrorCode, String) {
    match rebuild(certificate) {
        Ok(_) => panic!("the tampered certificate was accepted"),
        Err(e) => (e.code, e.message),
    }
}

fn entry_index(certificate: &EnvironmentCertificate, name: &str) -> usize {
    certificate
        .entries
        .iter()
        .position(|e| e.names().contains(&name))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

fn push_node(certificate: &mut EnvironmentCertificate, node: TermNode) -> u32 {
    certificate.nodes.push(node);
    (certificate.nodes.len() - 1) as u32
}

// -------------------------------------------------------------------------------------------------
// Capture and rebuild agree with the importer.
// -------------------------------------------------------------------------------------------------

#[test]
fn every_t06_002_root_captures_and_rebuilds_to_its_imported_statement() {
    let (ctx, _) = adapter::import_audited(T06_002).unwrap();
    for short in T06_ROOTS {
        let target = t06(short);
        let certificate = capture(T06_002, &target);
        assert_eq!(certificate.schema, ENVIRONMENT_SCHEMA);
        assert_eq!(certificate.prelude_version, PRELUDE_VERSION);
        assert_eq!(certificate.base, EnvironmentBase::Kernel);
        assert_eq!(certificate.conversion, ConversionDiscipline::Exact);
        assert_eq!(certificate.target, target);
        let rebuilt = rebuild(&certificate).unwrap_or_else(|e| panic!("{short}: {e}"));
        let imported = ctx.get_global(&target).cloned().unwrap();
        assert!(
            defeq_for_test(&ctx, &rebuilt.target_type, &imported),
            "{short}: statement"
        );
        assert!(
            rebuilt.axioms.axioms.is_empty(),
            "{short} rests on no axiom: {:?}",
            rebuilt.axioms
        );
        assert!(
            rebuilt.axioms.unresolved.is_empty(),
            "{short}: {:?}",
            rebuilt.axioms.unresolved
        );
        assert!(
            rebuilt.assumptions.is_empty(),
            "{short}: {:?}",
            rebuilt.assumptions
        );
        assert_eq!(
            rebuilt.target_is_theorem,
            short != "bigMotive",
            "{short}: the theorem/definition kind is preserved"
        );
        assert!(rebuilt.context.pending_obligations().is_empty());
        // The proof in the certificate is the importer's checked body, not a stand-in.
        let (retained, _) = adapter::import_audited_retaining_source(T06_002, &target).unwrap();
        let (_, _, body) = retained.retained_source().unwrap();
        assert_eq!(
            &rebuilt.target_proof, body,
            "{short}: the proof is carried exactly"
        );
        // A theorem is published as Lean publishes it — unfoldable, its body the checked proof —
        // and a definition keeps its body; both carry exactly the certificate's proof.
        assert_eq!(
            rebuilt.context.get_definition_body(&target),
            Some(&rebuilt.target_proof),
            "{short}"
        );
        assert!(!rebuilt.independent.is_empty());
    }
}

#[test]
fn the_rebuilt_environment_declares_exactly_what_the_certificate_lists() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let rebuilt = rebuild(&certificate).unwrap();
    let (ctx, _) = adapter::import_audited(T06_002).unwrap();
    for entry in &certificate.entries {
        for name in entry.names() {
            let rebuilt_ty = rebuilt
                .context
                .poly_declaration(name)
                .map(|(_, t)| t.clone())
                .or_else(|| rebuilt.context.get_global(name).cloned())
                .unwrap_or_else(|| panic!("{name} missing after rebuild"));
            let imported_ty = ctx
                .poly_declaration(name)
                .map(|(_, t)| t.clone())
                .or_else(|| ctx.get_global(name).cloned())
                .unwrap_or_else(|| panic!("{name} missing from the import"));
            assert!(defeq_for_test(&ctx, &rebuilt_ty, &imported_ty), "{name}");
        }
    }
    assert!(certificate
        .entries
        .iter()
        .any(|e| matches!(e, EnvironmentEntry::Inductive { types, .. } if types[0].name == "Nat")));
    assert!(certificate.entries.iter().any(
        |e| matches!(e, EnvironmentEntry::Inductive { types, .. } if types[0].name == "/s:\"Eq\"")
    ));
}

#[test]
fn a_structure_rebuilds_with_the_eta_its_proof_needs() {
    let certificate = capture(STRUCT_ETA, "/s:\"structEta\"");
    let at = certificate
        .entries
        .iter()
        .position(|e| matches!(e, EnvironmentEntry::Structure { .. }))
        .expect("the Prod structure's η is part of the environment");
    let EnvironmentEntry::Structure {
        inductive,
        projections,
    } = &certificate.entries[at]
    else {
        unreachable!()
    };
    assert_eq!(inductive, "/s:\"Prod\"");
    assert_eq!(projections.len(), 2);
    let target = entry_index(&certificate, "/s:\"structEta\"");
    assert!(
        at < target,
        "η is registered before the proof that needs it"
    );
    let rebuilt = rebuild(&certificate).unwrap_or_else(|e| panic!("{e}"));
    let (ctx, _) = adapter::import_audited(STRUCT_ETA).unwrap();
    let (imported, replayed) = (
        ctx.struct_info("/s:\"Prod\"").unwrap(),
        rebuilt.context.struct_info("/s:\"Prod\"").unwrap(),
    );
    assert_eq!(
        (&imported.mk, imported.num_params, &imported.projections),
        (&replayed.mk, replayed.num_params, &replayed.projections)
    );
    // Without η the same proof does not check: the entry is load-bearing, not decoration.
    let mut without = certificate.clone();
    without.entries.remove(at);
    assert_eq!(refusal(&without).0, EnvironmentErrorCode::IllTyped);
}

#[test]
fn structure_eta_is_refused_when_its_projections_do_not_compute() {
    let certificate = capture(STRUCT_ETA, "/s:\"structEta\"");
    let at = certificate
        .entries
        .iter()
        .position(|e| matches!(e, EnvironmentEntry::Structure { .. }))
        .unwrap();
    let mut swapped = certificate.clone();
    if let EnvironmentEntry::Structure { projections, .. } = &mut swapped.entries[at] {
        projections.swap(0, 1);
    }
    let (code, message) = refusal(&swapped);
    assert_eq!(code, EnvironmentErrorCode::IllTyped, "{message}");
    assert!(message.contains("does not compute"), "{message}");
    let mut short = certificate.clone();
    if let EnvironmentEntry::Structure { projections, .. } = &mut short.entries[at] {
        projections.pop();
    }
    assert_eq!(refusal(&short).0, EnvironmentErrorCode::IllTyped);
}

#[test]
fn quotient_primitives_rebuild_through_their_certified_installation() {
    let certificate = capture(QUOT_LIFT, "/s:\"quotLiftReduction\"");
    let quotients: Vec<_> = certificate
        .entries
        .iter()
        .filter(|e| matches!(e, EnvironmentEntry::Quotient { .. }))
        .collect();
    assert!(quotients.len() >= 3, "{quotients:?}");
    rebuild(&certificate).unwrap_or_else(|e| panic!("{e}"));
    // A quotient primitive with a forged signature is refused, whatever it is called.
    let mut forged = certificate.clone();
    let prop = push_node(&mut forged, TermNode::Sort(Universe::Prop));
    let at = forged
        .entries
        .iter()
        .position(|e| matches!(e, EnvironmentEntry::Quotient { .. }))
        .unwrap();
    if let EnvironmentEntry::Quotient { ty, .. } = &mut forged.entries[at] {
        *ty = prop;
    }
    assert_eq!(refusal(&forged).0, EnvironmentErrorCode::IllTyped);
    // An axiom may not take a quotient primitive's name.
    let mut impostor = certificate.clone();
    if let EnvironmentEntry::Quotient {
        level_params, ty, ..
    } = impostor.entries[at].clone()
    {
        impostor.entries[at] = EnvironmentEntry::Axiom {
            name: "kernel.quot.type".into(),
            level_params,
            ty,
        };
    }
    assert_eq!(
        refusal(&impostor).0,
        EnvironmentErrorCode::TrustedNameReplacement
    );
}

#[test]
fn a_mutual_polymorphic_block_rebuilds_as_one_block() {
    let certificate = capture(MUTUAL, "/s:\"arenaSingleton\"");
    let block = certificate
        .entries
        .iter()
        .find_map(|e| match e {
            EnvironmentEntry::Inductive {
                types,
                level_params,
            } if types.len() == 2 => Some((types, level_params)),
            _ => None,
        })
        .expect("ArenaTree and ArenaForest are one mutual entry");
    assert_eq!(block.1.len(), 1, "the block keeps its universe parameter");
    let rebuilt = rebuild(&certificate).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        rebuilt
            .context
            .mutual_block_of("/s:\"ArenaTree\"")
            .map(|b| b.len()),
        Some(2)
    );
}

#[test]
fn an_indexed_family_rebuilds() {
    let certificate = capture(RB_TREE, "/s:\"rbTreeDef\"");
    rebuild(&certificate).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn a_certificate_round_trips_through_json_exactly() {
    for (export, target) in [
        (T06_002, t06("viaMpr")),
        (STRUCT_ETA, "/s:\"structEta\"".into()),
        (QUOT_LIFT, "/s:\"quotLiftReduction\"".into()),
    ] {
        let certificate = capture(export, &target);
        let json = serde_json::to_string(&certificate).unwrap();
        let AnyCertificate::Environment(read) = read_certificate(&json).unwrap() else {
            panic!("dispatch")
        };
        assert_eq!(read, certificate);
        assert_eq!(
            serde_json::to_string(&read).unwrap(),
            json,
            "serialization is deterministic"
        );
    }
}

#[test]
fn capture_is_deterministic() {
    let a = serde_json::to_string(&capture(T06_002, &t06("jDependent"))).unwrap();
    let b = serde_json::to_string(&capture(T06_002, &t06("jDependent"))).unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_term_table_shares_each_distinct_subterm_once() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let rows: std::collections::HashSet<String> = certificate
        .nodes
        .iter()
        .map(|n| serde_json::to_string(n).unwrap())
        .collect();
    assert_eq!(
        rows.len(),
        certificate.nodes.len(),
        "no row is written twice"
    );
}

// -------------------------------------------------------------------------------------------------
// The format: schema, prelude, unknown fields.
// -------------------------------------------------------------------------------------------------

#[test]
fn unknown_fields_are_refused_at_every_level() {
    let certificate = capture(T06_002, &t06("symmViaRec"));
    let json: serde_json::Value = serde_json::to_value(&certificate).unwrap();
    let mut probes = Vec::new();
    let mut top = json.clone();
    top["extension"] = serde_json::json!(true);
    probes.push(("top level", top));
    let mut entry = json.clone();
    entry["entries"][0]["extension"] = serde_json::json!(1);
    probes.push(("entry", entry));
    let inductive_at = certificate
        .entries
        .iter()
        .position(|e| matches!(e, EnvironmentEntry::Inductive { .. }))
        .unwrap();
    let mut member = json.clone();
    member["entries"][inductive_at]["types"][0]["extension"] = serde_json::json!(1);
    probes.push(("inductive member", member));
    let mut constructor = json.clone();
    constructor["entries"][inductive_at]["types"][0]["constructors"][0]["extension"] =
        serde_json::json!(1);
    probes.push(("constructor", constructor));
    let mut kind = json.clone();
    kind["entries"][0]["kind"] = serde_json::json!("oracle");
    probes.push(("entry kind", kind));
    let mut node = json.clone();
    node["nodes"][0] = serde_json::json!({"wildcard": 0});
    probes.push(("term row", node));
    for (what, probe) in probes {
        let error = read_certificate(&probe.to_string()).expect_err(what);
        assert_eq!(
            error.code,
            EnvironmentErrorCode::Malformed,
            "{what}: {error}"
        );
    }
}

#[test]
fn a_theorem_certificate_is_never_read_as_a_bare_one() {
    let theorem = TheoremCertificate::new(
        "t",
        Term::Var("h".into()),
        Term::Global("P".into()),
        vec![CertDeclaration {
            name: "P".into(),
            ty: Term::Sort(Universe::Prop),
        }],
        vec![],
    );
    let json = serde_json::to_string(&theorem).unwrap();
    assert!(
        serde_json::from_str::<Certificate>(&json).is_err(),
        "a theorem certificate's delta is never dropped"
    );
    assert!(matches!(read_certificate(&json).unwrap(), AnyCertificate::Theorem(t) if t == theorem));
    let bare = Certificate::new(Term::Sort(Universe::Prop), Term::Sort(Universe::Type(0)));
    let json = serde_json::to_string(&bare).unwrap();
    assert!(matches!(read_certificate(&json).unwrap(), AnyCertificate::Proof(c) if c == bare));
    let mut extended: serde_json::Value = serde_json::from_str(&json).unwrap();
    extended["declarations"] = serde_json::json!([]);
    assert_eq!(
        read_certificate(&extended.to_string()).unwrap_err().code,
        EnvironmentErrorCode::Malformed
    );
}

#[test]
fn a_retired_prelude_is_an_explicit_incompatibility_for_every_certificate_kind() {
    let environment = capture(T06_002, &t06("symmViaRec"));
    let bare = Certificate::new(Term::Sort(Universe::Prop), Term::Sort(Universe::Type(0)));
    let theorem = TheoremCertificate::new(
        "t",
        Term::Sort(Universe::Prop),
        Term::Sort(Universe::Type(0)),
        vec![],
        vec![],
    );
    let documents = [
        serde_json::to_value(&environment).unwrap(),
        serde_json::to_value(&bare).unwrap(),
        serde_json::to_value(&theorem).unwrap(),
    ];
    for mut document in documents {
        document["prelude_version"] = serde_json::json!("logos-coc-1");
        let error = read_certificate(&document.to_string()).unwrap_err();
        assert_eq!(
            error.code,
            EnvironmentErrorCode::IncompatiblePrelude,
            "{error}"
        );
        assert!(
            error.message.contains("logos-coc-1") && error.message.contains("retired"),
            "{error}"
        );
        assert!(error.message.contains("no validated migration"), "{error}");
        assert!(error.message.contains(PRELUDE_VERSION), "{error}");
    }
    let mut unknown = serde_json::to_value(&environment).unwrap();
    unknown["prelude_version"] = serde_json::json!("logos-coc-9");
    let error = read_certificate(&unknown.to_string()).unwrap_err();
    assert_eq!(error.code, EnvironmentErrorCode::IncompatiblePrelude);
    assert!(
        !error.message.contains("retired"),
        "an unknown tag is not called retired: {error}"
    );
    // The rebuild re-checks the tag itself; a certificate built in memory cannot skip the gate.
    let mut in_memory = environment.clone();
    in_memory.prelude_version = "logos-coc-1".into();
    assert_eq!(
        refusal(&in_memory).0,
        EnvironmentErrorCode::IncompatiblePrelude
    );
    assert!(check_prelude_version(PRELUDE_VERSION).is_ok());
}

#[test]
fn an_unknown_schema_is_unsupported_never_guessed() {
    let mut document = serde_json::to_value(capture(T06_002, &t06("symmViaRec"))).unwrap();
    document["schema"] = serde_json::json!("logos-environment-certificate/3");
    assert_eq!(
        read_certificate(&document.to_string()).unwrap_err().code,
        EnvironmentErrorCode::UnsupportedSchema
    );
    let mut in_memory = capture(T06_002, &t06("symmViaRec"));
    in_memory.schema = "logos-environment-certificate/0".into();
    assert_eq!(
        refusal(&in_memory).0,
        EnvironmentErrorCode::UnsupportedSchema
    );
}

// -------------------------------------------------------------------------------------------------
// Admission: the table and the limits, before any work.
// -------------------------------------------------------------------------------------------------

#[test]
fn forward_and_self_references_in_the_term_table_are_malformed() {
    let certificate = capture(T06_002, &t06("symmViaRec"));
    let last = certificate.nodes.len() as u32;
    for bad in [
        TermNode::App(last, 0),
        TermNode::App(0, last + 7),
        TermNode::Pi("x".into(), 0, last),
    ] {
        let mut tampered = certificate.clone();
        tampered.nodes.push(bad.clone());
        let (code, message) = refusal(&tampered);
        assert_eq!(code, EnvironmentErrorCode::Malformed, "{bad:?}: {message}");
    }
    let mut dangling = certificate.clone();
    if let EnvironmentEntry::Theorem { proof, .. } = dangling.entries.last_mut().unwrap() {
        *proof = 9_999_999;
    }
    assert_eq!(refusal(&dangling).0, EnvironmentErrorCode::Malformed);
}

#[test]
fn every_limit_is_enforced_before_any_term_is_built() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let generous = EnvironmentLimits::default();
    certificate.rebuild(&generous).unwrap();
    let rows = certificate.nodes.len();
    for (what, limits) in [
        (
            "rows",
            EnvironmentLimits {
                max_nodes: rows - 1,
                ..generous
            },
        ),
        (
            "entries",
            EnvironmentLimits {
                max_entries: certificate.entries.len() - 1,
                ..generous
            },
        ),
        (
            "depth",
            EnvironmentLimits {
                max_depth: 3,
                ..generous
            },
        ),
        (
            "tree size",
            EnvironmentLimits {
                max_tree_nodes: 10,
                ..generous
            },
        ),
        (
            "name bytes",
            EnvironmentLimits {
                max_name_bytes: 4,
                ..generous
            },
        ),
    ] {
        let error = certificate
            .rebuild(&limits)
            .err()
            .unwrap_or_else(|| panic!("{what}: accepted"));
        assert_eq!(
            error.code,
            EnvironmentErrorCode::ResourceLimit,
            "{what}: {error}"
        );
    }
    // An exponential DAG: sixty-four doublings of one application are refused by size, never built.
    let mut bomb = certificate.clone();
    let mut top = push_node(&mut bomb, TermNode::Global("Nat".into()));
    for _ in 0..64 {
        top = push_node(&mut bomb, TermNode::App(top, top));
    }
    bomb.entries.push(EnvironmentEntry::Axiom {
        name: "/s:\"bomb\"".into(),
        level_params: vec![],
        ty: top,
    });
    let error = bomb
        .rebuild(&EnvironmentLimits {
            max_depth: 1000,
            ..generous
        })
        .err()
        .unwrap();
    assert_eq!(error.code, EnvironmentErrorCode::ResourceLimit, "{error}");
}

// -------------------------------------------------------------------------------------------------
// Rebuild: every entry kernel-validated.
// -------------------------------------------------------------------------------------------------

#[test]
fn a_changed_proof_body_is_refused_by_the_kernel() {
    let certificate = capture(T06_002, &t06("symmViaRec"));
    let mut tampered = certificate.clone();
    // A declared term of the wrong type: `Eq.refl` itself, where a proof of symmetry belongs.
    let wrong = push_node(
        &mut tampered,
        TermNode::Const("/s:\"Eq\"/s:\"refl\"".into(), vec![Universe::Type(0)]),
    );
    let at = entry_index(&tampered, &t06("symmViaRec"));
    if let EnvironmentEntry::Theorem { proof, .. } = &mut tampered.entries[at] {
        *proof = wrong;
    } else {
        panic!("symmViaRec is a theorem");
    }
    let (code, message) = refusal(&tampered);
    assert_eq!(code, EnvironmentErrorCode::IllTyped, "{message}");
}

#[test]
fn a_definition_with_a_false_declared_type_is_refused() {
    let certificate = capture(T06_002, &t06("bigMotive"));
    let mut tampered = certificate.clone();
    // `bigMotive` claimed to have type `Nat`.
    let nat = push_node(&mut tampered, TermNode::Global("Nat".into()));
    let at = entry_index(&tampered, &t06("bigMotive"));
    if let EnvironmentEntry::Definition { ty, .. } = &mut tampered.entries[at] {
        *ty = nat;
    } else {
        panic!("bigMotive is a definition");
    }
    assert_eq!(refusal(&tampered).0, EnvironmentErrorCode::IllTyped);
}

#[test]
fn a_removed_definition_leaves_an_unresolved_obligation() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let target = entry_index(&certificate, &t06("viaMpr"));
    let removable = certificate.entries[..target]
        .iter()
        .rposition(|e| {
            matches!(
                e,
                EnvironmentEntry::Definition { .. } | EnvironmentEntry::Theorem { .. }
            )
        })
        .expect("viaMpr depends on at least one definition or lemma");
    let mut tampered = certificate.clone();
    tampered.entries.remove(removable);
    assert_eq!(
        refusal(&tampered).0,
        EnvironmentErrorCode::UnresolvedObligation
    );
}

#[test]
fn a_forward_reference_is_an_unresolved_obligation_never_a_deferral() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let target = entry_index(&certificate, &t06("viaMpr"));
    let mut tampered = certificate.clone();
    let moved = tampered.entries.remove(target);
    tampered.entries.insert(0, moved);
    let (code, message) = refusal(&tampered);
    assert_eq!(
        code,
        EnvironmentErrorCode::UnresolvedObligation,
        "{message}"
    );
    assert!(message.contains("not declared before it"), "{message}");
}

#[test]
fn a_declaration_cycle_is_named_as_a_cycle() {
    let mut certificate = capture(T06_002, &t06("symmViaRec"));
    let nat = push_node(&mut certificate, TermNode::Global("Nat".into()));
    let a = push_node(
        &mut certificate,
        TermNode::Global("/s:\"cycle\"/s:\"b\"".into()),
    );
    let b = push_node(
        &mut certificate,
        TermNode::Global("/s:\"cycle\"/s:\"a\"".into()),
    );
    certificate.entries.push(EnvironmentEntry::Definition {
        name: "/s:\"cycle\"/s:\"a\"".into(),
        level_params: vec![],
        ty: nat,
        body: a,
    });
    certificate.entries.push(EnvironmentEntry::Definition {
        name: "/s:\"cycle\"/s:\"b\"".into(),
        level_params: vec![],
        ty: nat,
        body: b,
    });
    let (code, message) = refusal(&certificate);
    assert_eq!(code, EnvironmentErrorCode::DependencyCycle, "{message}");
    // A self-citing definition is a cycle of one.
    let mut own = capture(T06_002, &t06("symmViaRec"));
    let nat = push_node(&mut own, TermNode::Global("Nat".into()));
    let me = push_node(&mut own, TermNode::Global("/s:\"loop\"".into()));
    own.entries.push(EnvironmentEntry::Definition {
        name: "/s:\"loop\"".into(),
        level_params: vec![],
        ty: nat,
        body: me,
    });
    assert_eq!(
        refusal(&own).0,
        EnvironmentErrorCode::UnresolvedObligation,
        "a self-reference is never declared before itself"
    );
}

#[test]
fn trusted_and_reserved_names_are_never_replaced() {
    let certificate = capture(T06_002, &t06("symmViaRec"));
    let prop = |c: &mut EnvironmentCertificate| push_node(c, TermNode::Sort(Universe::Prop));
    for name in [
        "Quot_lift",
        "dne",
        "le",
        "Eq",
        "try_script",
        "kernel.quot.lift",
        "Zero",
    ] {
        let mut tampered = certificate.clone();
        let ty = prop(&mut tampered);
        tampered.entries.insert(
            0,
            EnvironmentEntry::Axiom {
                name: name.into(),
                level_params: vec![],
                ty,
            },
        );
        let (code, message) = refusal(&tampered);
        assert_eq!(
            code,
            EnvironmentErrorCode::TrustedNameReplacement,
            "{name}: {message}"
        );
    }
    // A name declared twice.
    let mut twice = certificate.clone();
    let first = twice
        .entries
        .iter()
        .find_map(|e| match e {
            EnvironmentEntry::Theorem { name, .. } => Some(name.clone()),
            _ => None,
        })
        .unwrap();
    let ty = prop(&mut twice);
    twice.entries.push(EnvironmentEntry::Axiom {
        name: first,
        level_params: vec![],
        ty,
    });
    assert_eq!(
        refusal(&twice).0,
        EnvironmentErrorCode::TrustedNameReplacement
    );
    // On the standard-library base the prelude's own declarations are the trusted names.
    let mut on_prelude = capture(T06_002, &t06("viaMpr"));
    on_prelude.base = EnvironmentBase::StandardLibrary;
    assert_eq!(
        refusal(&on_prelude).0,
        EnvironmentErrorCode::TrustedNameReplacement,
        "Nat is the prelude's"
    );
}

#[test]
fn a_noncanonical_nat_is_refused() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let at = certificate
        .entries
        .iter()
        .position(
            |e| matches!(e, EnvironmentEntry::Inductive { types, .. } if types[0].name == "Nat"),
        )
        .unwrap();
    // Nat with its successor removed: a one-element type the kernel's literals would lie about.
    let mut truncated = certificate.clone();
    if let EnvironmentEntry::Inductive { types, .. } = &mut truncated.entries[at] {
        types[0].constructors.pop();
    }
    assert_eq!(
        refusal(&truncated).0,
        EnvironmentErrorCode::TrustedNameReplacement
    );
    // Nat at the wrong universe.
    let mut lifted = certificate.clone();
    let type1 = push_node(&mut lifted, TermNode::Sort(Universe::Type(1)));
    if let EnvironmentEntry::Inductive { types, .. } = &mut lifted.entries[at] {
        types[0].arity = type1;
    }
    assert_eq!(
        refusal(&lifted).0,
        EnvironmentErrorCode::TrustedNameReplacement
    );
}

#[test]
fn an_altered_constructor_or_universe_is_refused() {
    let certificate = capture(T06_002, &t06("symmViaRec"));
    let at = certificate.entries.iter().position(|e| matches!(e, EnvironmentEntry::Inductive { types, .. } if types[0].name == "/s:\"Eq\"")).unwrap();
    // Eq's universe parameter renamed in the telescope but not in the terms: unbound level.
    let mut renamed = certificate.clone();
    if let EnvironmentEntry::Inductive { level_params, .. } = &mut renamed.entries[at] {
        level_params[0] = "/s:\"v\"".into();
    }
    assert_eq!(refusal(&renamed).0, EnvironmentErrorCode::IllTyped);
    // Eq.refl claimed to have type `Eq` itself — an unapplied family, not a type.
    let mut bent = certificate.clone();
    let family = push_node(&mut bent, TermNode::Global("/s:\"Eq\"".into()));
    if let EnvironmentEntry::Inductive { types, .. } = &mut bent.entries[at] {
        types[0].constructors[0].ty = family;
    }
    assert_eq!(refusal(&bent).0, EnvironmentErrorCode::IllTyped);
}

#[test]
fn holes_are_refused_anywhere_in_the_table() {
    let mut certificate = capture(T06_002, &t06("symmViaRec"));
    push_node(&mut certificate, TermNode::Hole);
    assert_eq!(refusal(&certificate).0, EnvironmentErrorCode::IllTyped);
}

#[test]
fn the_target_is_proved_never_assumed() {
    let mut certificate = capture(T06_002, &t06("symmViaRec"));
    let at = entry_index(&certificate, &t06("symmViaRec"));
    if let EnvironmentEntry::Theorem {
        name,
        level_params,
        ty,
        ..
    } = certificate.entries[at].clone()
    {
        certificate.entries[at] = EnvironmentEntry::Axiom {
            name,
            level_params,
            ty,
        };
    }
    let (code, message) = refusal(&certificate);
    assert_eq!(code, EnvironmentErrorCode::TargetNotProved, "{message}");
    assert!(message.contains("assumption"), "{message}");
    let mut missing = capture(T06_002, &t06("symmViaRec"));
    missing.target = "/s:\"nowhere\"".into();
    assert_eq!(refusal(&missing).0, EnvironmentErrorCode::TargetNotProved);
}

#[test]
fn a_theorem_is_captured_with_its_published_proof_and_an_opaque_header_is_not_a_proof() {
    let (ctx, _) = adapter::import_audited(T06_002).unwrap();
    let none = std::collections::BTreeMap::new();
    let request = |context: &logicaffeine_kernel::Context, target: &str| {
        EnvironmentCertificate::capture(&logicaffeine_kernel::certificate::CaptureRequest {
            context,
            base: EnvironmentBase::Kernel,
            conversion: ConversionDiscipline::Exact,
            target,
            opaque_proofs: &none,
        })
    };
    // A theorem unfolds like Lean's, so its proof is in the context: no retention is needed, and
    // the certificate carries it as a Theorem entry with exactly the importer's checked proof.
    let target = t06("symmViaRec");
    let certificate = request(&ctx, &target).expect("a published theorem carries its proof");
    let at = entry_index(&certificate, &target);
    assert!(
        matches!(certificate.entries[at], EnvironmentEntry::Theorem { .. }),
        "a theorem stays a theorem"
    );
    let rebuilt = rebuild(&certificate).unwrap();
    assert_eq!(
        Some(&rebuilt.target_proof),
        ctx.get_definition_body(&target)
    );
    // A genuinely opaque declaration keeps only its header, and a header is not a proof.
    let mut sealed = (*ctx).clone();
    let statement = ctx.get_global(&target).unwrap().clone();
    let proof = ctx.get_definition_body(&target).unwrap().clone();
    sealed
        .declare_opaque_checked("sealed_symm", vec![], statement, proof)
        .unwrap();
    let error = request(&sealed, "sealed_symm")
        .err()
        .expect("an opaque header is not its proof");
    assert_eq!(error.code, EnvironmentErrorCode::MissingEvidence, "{error}");
}

#[test]
fn a_cumulative_only_declaration_is_refused_under_exact_conversion() {
    // `lifted : Type 1 := Nat` checks only because LOGOS is cumulative; Lean would refuse it.
    let mut certificate = capture(T06_002, &t06("viaMpr"));
    let type1 = push_node(&mut certificate, TermNode::Sort(Universe::Type(1)));
    let nat = push_node(&mut certificate, TermNode::Global("Nat".into()));
    certificate.entries.push(EnvironmentEntry::Definition {
        name: "/s:\"lifted\"".into(),
        level_params: vec![],
        ty: type1,
        body: nat,
    });
    let (code, message) = refusal(&certificate);
    assert_eq!(code, EnvironmentErrorCode::InexactConversion, "{message}");
    let mut cumulative = certificate.clone();
    cumulative.conversion = ConversionDiscipline::Cumulative;
    rebuild(&cumulative).unwrap_or_else(|e| panic!("the cumulative discipline accepts it: {e}"));
}

#[test]
fn transitive_assumptions_see_through_opaque_lemmas_and_aliases() {
    // sorryAx; lemma : P := sorryAx P false (opaque); alias := lemma (definition); T : P := alias.
    let mut e = Export::new();
    let (sorry, bool_false) = declare_sorry(&mut e);
    let p = e.name("P", 0);
    let lemma = e.name("lemma", 0);
    let alias = e.name("alias", 0);
    let t = e.name("T", 0);
    let prop = e.sort(0);
    e.axiom(p, prop, &[]);
    let pc = e.constant(p, &[]);
    let sorry_at_prop = e.constant(sorry, &[0]);
    let sorry_p = e.app(sorry_at_prop, pc);
    let false_c = e.constant(bool_false, &[]);
    let admitted = e.app(sorry_p, false_c);
    e.thm(lemma, pc, admitted);
    let lemma_c = e.constant(lemma, &[]);
    e.def(alias, pc, lemma_c);
    let alias_c = e.constant(alias, &[]);
    e.thm(t, pc, alias_c);
    let export = e.text();
    let certificate = capture(&export, "/s:\"T\"");
    let rebuilt = rebuild(&certificate).unwrap_or_else(|err| panic!("{err}"));
    assert!(
        rebuilt.axioms.axioms.contains("/s:\"sorryAx\""),
        "{:?}",
        rebuilt.axioms
    );
    assert!(
        rebuilt.axioms.axioms.contains("/s:\"P\""),
        "{:?}",
        rebuilt.axioms
    );
    let names: Vec<&str> = rebuilt
        .assumptions
        .iter()
        .map(|(n, _, _)| n.as_str())
        .collect();
    assert!(names.contains(&"/s:\"sorryAx\""), "{names:?}");
}

#[test]
fn independent_rechecking_is_recorded_per_declaration() {
    let certificate = capture(T06_002, &t06("viaMpr"));
    let rebuilt = rebuild(&certificate).unwrap();
    let declared: usize = certificate.entries.iter().map(|e| e.names().len()).sum();
    assert!(
        rebuilt.independent.len() >= declared - 1,
        "{} verdicts for {declared} names",
        rebuilt.independent.len()
    );
    assert!(rebuilt
        .independent
        .iter()
        .any(|(n, v)| n == &t06("viaMpr") && *v == IndependentVerdict::Agreed));
    assert_eq!(
        rebuilt.independently_agreed(),
        rebuilt
            .independent
            .iter()
            .all(|(_, v)| *v == IndependentVerdict::Agreed)
    );
}

#[test]
fn the_smallest_honest_environment_reports_its_one_assumption_pair() {
    let certificate = capture(&small_theorem().text(), "/s:\"T\"");
    let rebuilt = rebuild(&certificate).unwrap();
    let names: Vec<&str> = rebuilt
        .assumptions
        .iter()
        .map(|(n, _, _)| n.as_str())
        .collect();
    assert_eq!(names, vec!["/s:\"P\"", "/s:\"p\""]);
    assert!(rebuilt.axioms.axioms.contains("/s:\"p\""));
    assert!(capture_result(&small_theorem().text(), "/s:\"nothing\"").is_err());
}

#[test]
fn source_binder_annotations_travel_with_the_certificate() {
    let (ctx, _) = adapter::import_audited(T06_002).unwrap();
    let certificate = capture(T06_002, &t06("viaMpr"));
    assert!(
        !certificate.binder_kinds.is_empty(),
        "the export's implicit binders are recorded"
    );
    let rebuilt = rebuild(&certificate).unwrap();
    for entry in &certificate.entries {
        for name in entry.names() {
            assert_eq!(
                rebuilt.context.binder_kinds(name),
                ctx.binder_kinds(name),
                "{name}"
            );
        }
    }
    // An annotation for a declaration the certificate does not make is refused.
    let mut stray = certificate.clone();
    stray.binder_kinds.insert(
        "/s:\"elsewhere\"".into(),
        vec![logicaffeine_kernel::certificate::BinderKind::Implicit],
    );
    assert_eq!(refusal(&stray).0, EnvironmentErrorCode::Malformed);
    // More annotations than the declaration has binders is refused.
    let mut long = certificate.clone();
    let name = long.binder_kinds.keys().next().unwrap().clone();
    long.binder_kinds
        .get_mut(&name)
        .unwrap()
        .extend(std::iter::repeat(logicaffeine_kernel::certificate::BinderKind::Explicit).take(64));
    assert_eq!(refusal(&long).0, EnvironmentErrorCode::Malformed);
}
