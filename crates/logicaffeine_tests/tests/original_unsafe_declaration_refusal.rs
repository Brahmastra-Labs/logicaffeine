//! An `unsafe def` is refused, as Lean's kernel keeps it out of the logic; its safe partner imports.
//!
//! Lean's kernel admits an `unsafe` declaration only to an unsafe compartment that no safe
//! declaration may reference ("invalid declaration, it uses unsafe declaration"): it is checked
//! neither for termination (`diverge n := diverge n`) nor across `unsafeCast`, and it has no
//! logical content. LOGOS has no such compartment, and every registration door is trusted, so the
//! reader quarantines an unsafe declaration: it reserves the name, installs nothing, and refuses an
//! import whose target is unsafe. `@[implemented_by fastIdImpl] def safeId` carries only its safe
//! body; the implementation is never read.
//!
//! Original witness: `fixtures/lean_semantics/LK_UnsafeRuntime` — the corpus's unchanged case
//! T02-008 (`crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/t02_declarations/T02-008_unsafe_def.lean`, `@expect reject`),
//! exported by Lean 4.14.0 at the roots `diverge`, `fastIdImpl`, `safeId`, `safeId_eq`.
use logicaffeine_kernel::interface::lean::{
    self as adapter, emit, reference, DiagnosticCode, Verdict,
};
use serde_json::Value;

const WITNESS: &str = include_str!("fixtures/lean_semantics/LK_UnsafeRuntime.ndjson");
const WITNESS_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LK_UnsafeRuntime.provenance.json");
const WITNESS_SOURCE: &str = include_str!("fixtures/lean_semantics/LK_UnsafeRuntime.lean");

const UNSAFE: [&str; 2] = ["T02_008.diverge", "T02_008.fastIdImpl"];

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

fn rows() -> Vec<Value> {
    WITNESS
        .lines()
        .map(|l| serde_json::from_str(l).expect("ndjson row"))
        .collect()
}

/// The export row declaring `dotted`, as `(line index, record kind)`.
fn declaration_row(rows: &[Value], dotted: &str) -> (usize, &'static str) {
    let names: std::collections::HashMap<u64, (u64, String)> = rows
        .iter()
        .filter_map(|r| {
            Some((
                r["in"].as_u64()?,
                (
                    r["str"]["pre"].as_u64()?,
                    r["str"]["str"].as_str()?.to_string(),
                ),
            ))
        })
        .collect();
    let spell = |mut id: u64| {
        let mut parts = Vec::new();
        while let Some((pre, s)) = names.get(&id) {
            parts.push(s.clone());
            id = *pre;
        }
        parts.reverse();
        parts.join(".")
    };
    rows.iter()
        .enumerate()
        .find_map(|(i, r)| {
            ["def", "thm"].into_iter().find_map(|k| {
                let id = r.get(k)?["name"].as_u64()?;
                (spell(id) == dotted).then_some((i, k))
            })
        })
        .unwrap_or_else(|| panic!("{dotted} is declared in the witness"))
}

#[test]
fn the_witness_is_the_corpus_case_exported_by_lean_4_14_with_its_unsafe_declarations() {
    let p: Value = serde_json::from_str(WITNESS_PROVENANCE).unwrap();
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["records"], WITNESS.lines().count());
    assert!(WITNESS_SOURCE.contains("@expect    reject"));
    assert!(WITNESS_SOURCE.contains("unsafe def diverge (n : Nat) : Nat := diverge n"));
    assert!(WITNESS_SOURCE.contains("@[implemented_by fastIdImpl]"));
    let rows = rows();
    for name in UNSAFE {
        let (i, kind) = declaration_row(&rows, name);
        assert_eq!(
            (kind, rows[i][kind]["safety"].as_str()),
            ("def", Some("unsafe")),
            "{name} is exported unsafe"
        );
    }
    let (i, _) = declaration_row(&rows, "T02_008.safeId");
    assert_eq!(rows[i]["def"]["safety"], "safe");
}

#[test]
fn an_unsafe_target_is_refused_with_the_typed_unsupported_feature_code() {
    for root in UNSAFE {
        let target = n(root);
        let intake = adapter::intake::read(WITNESS, &adapter::intake::STAGE_A).expect("intake");
        let closure = intake
            .closure(&target)
            .unwrap_or_else(|e| panic!("{root}: closure: {e:?}"));
        match adapter::import_retaining_closure(WITNESS, &target, &closure) {
            Err(Verdict::Decline(reason)) => {
                assert_eq!(
                    reason.code(),
                    DiagnosticCode::UnsupportedFeature,
                    "{root}: {reason:?}"
                );
                assert!(
                    reason.message().contains("runtime declarations remain unchecked without a subsequent checked safe target"),
                    "{root}: {reason:?}"
                );
            }
            Err(other) => panic!(
                "{root}: an unsafe target is declined as unsupported, never rejected: {other:?}"
            ),
            Ok(_) => panic!("{root}: an unsafe declaration must never import"),
        }
    }
}

#[test]
fn the_whole_case_imports_its_safe_content_and_nothing_unsafe() {
    let (ctx, report) = adapter::import_audited(WITNESS)
        .unwrap_or_else(|v| panic!("the safe content imports: {v:?}"));
    for name in UNSAFE {
        let key = n(name);
        assert!(
            ctx.get_global(&key).is_none(),
            "{name}: no definition, no axiom"
        );
        assert!(
            ctx.poly_declaration(&key).is_none(),
            "{name}: not even a header"
        );
        let quarantined = report
            .runtime_declarations
            .iter()
            .find(|r| r.name == key)
            .unwrap_or_else(|| panic!("{name} is reported"));
        assert_eq!(
            (quarantined.kind.as_str(), quarantined.safety.as_str()),
            ("def", "unsafe"),
            "{name}"
        );
    }
    for (root, kind) in [("T02_008.safeId", "def"), ("T02_008.safeId_eq", "thm")] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(root))
            .unwrap_or_else(|| panic!("{root} audited"));
        assert_eq!(audit.kind, kind, "{root}");
        assert!(audit.axioms.is_empty(), "{root}: {:?}", audit.axioms);
    }
    let target = n("T02_008.safeId_eq");
    let intake = adapter::intake::read(WITNESS, &adapter::intake::STAGE_A).expect("intake");
    let closure = intake.closure(&target).expect("closure");
    assert!(
        !closure.iter().any(|c| UNSAFE.iter().any(|u| *c == n(u))),
        "the safe theorem's closure holds nothing unsafe"
    );
    let (imported, _) =
        adapter::import_retaining_closure(WITNESS, &target, &closure).expect("safeId_eq imports");
    let doc = emit::emit_document(&imported, &target).expect("emission");
    assert!(
        !UNSAFE
            .iter()
            .any(|u| doc.source.contains(u.rsplit('.').next().unwrap())),
        "no unsafe name reaches the document"
    );
    let original = reference::reference_declaration(WITNESS, &target).expect("reference");
    emit::check_statement(&doc, &original).unwrap_or_else(|f| panic!("safeId_eq: R8: {f}"));
}

#[test]
fn a_safe_declaration_that_uses_an_unsafe_one_is_rejected_as_lean_rejects_it() {
    // `safeId` given `diverge`'s body `fun n => diverge n`: a safe declaration referencing an unsafe
    // one, which Lean's kernel refuses ("invalid declaration, it uses unsafe declaration").
    let mut rows = rows();
    let (safe, _) = declaration_row(&rows, "T02_008.safeId");
    let (diverge, _) = declaration_row(&rows, "T02_008.diverge");
    rows[safe]["def"]["value"] = rows[diverge]["def"]["value"].clone();
    let forged: String = rows.iter().map(|r| r.to_string() + "\n").collect();
    match adapter::check_str(&forged) {
        Err(Verdict::Reject(reason)) => {
            assert!(
                reason
                    .message()
                    .contains("safe declaration references unsafe runtime declaration"),
                "{reason:?}"
            );
        }
        other => panic!("a safe declaration using an unsafe one must be rejected: {other:?}"),
    }
}
