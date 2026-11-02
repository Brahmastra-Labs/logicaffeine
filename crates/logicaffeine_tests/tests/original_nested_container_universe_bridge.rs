//! A nested family whose container instance lives in the family's universe, written differently.
//!
//! Lean's `elimNestedInductive` adds each container instance to the family's mutual block, and the
//! block's check compares result universes by level equivalence. `Lean.PersistentHashMap.Node.{u,v}
//! : Type (max u v)` nests through `Array (Entry α β (Node α β))`, and `Entry.{u,v,max u v} α β
//! (Node α β)` lives in `Type (max u v (max u v))`: the same universe, not the same expression.
//!
//! Original witness: `fixtures/lean_semantics/LK_NestedMaxLevel` (Lean 4.14.0; a mirror
//! `LK_NML.Node`, Lean's own `Lean.PersistentHashMap.Node`, and the control Lean's kernel refuses:
//! `BadT : Type 0` nesting through `LBox.{0} BadT : Type 1`, "mutually inductive types must live in
//! the same universe").
use logicaffeine_kernel::interface::lean::{
    self as adapter, emit, reference, DiagnosticCode, Verdict,
};
use serde_json::Value;

const WITNESS: &str = include_str!("fixtures/lean_semantics/LK_NestedMaxLevel.ndjson");
const WITNESS_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LK_NestedMaxLevel.provenance.json");
const WITNESS_SOURCE: &str = include_str!("fixtures/lean_semantics/LK_NestedMaxLevel.lean");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

/// The root's closure imports with the declaration audit (both kernels), its audit record is a
/// theorem or definition with no axioms beyond `allowed`, and its emitted statement is Lean's (R8).
fn bridged(export: &str, root: &str, kind: &str, allowed: &[&str]) {
    let target = n(root);
    let intake = adapter::intake::read(export, &adapter::intake::STAGE_A)
        .unwrap_or_else(|e| panic!("{root}: intake: {e:?}"));
    let closure = intake
        .closure(&target)
        .unwrap_or_else(|e| panic!("{root}: closure: {e:?}"));
    let (imported, report) = adapter::import_retaining_closure(export, &target, &closure)
        .unwrap_or_else(|e| panic!("{root}: import: {e:?}"));
    let audit = report
        .declarations
        .iter()
        .find(|d| d.name == target)
        .unwrap_or_else(|| panic!("{root}: audited"));
    assert_eq!(audit.kind, kind, "{root}");
    let allowed: Vec<String> = allowed.iter().map(|a| n(a)).collect();
    assert!(
        audit.axioms.iter().all(|a| allowed.contains(a)),
        "{root}: {:?}",
        audit.axioms
    );
    let doc = emit::emit_document(&imported, &target)
        .unwrap_or_else(|e| panic!("{root}: emission: {e:?}"));
    let original = reference::reference_declaration(export, &target)
        .unwrap_or_else(|e| panic!("{root}: reference: {e:?}"));
    emit::check_statement(&doc, &original).unwrap_or_else(|f| panic!("{root}: R8: {f}"));
}

#[test]
fn the_witness_is_the_pinned_lean_4_14_export_and_lean_refuses_a_nested_instance_in_another_universe(
) {
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
    let log = p["compile_log"].as_str().unwrap();
    assert!(log.contains("BadT: kernel REJECTED: (kernel) mutually inductive types must live in the same universe"), "{log}");
    assert!(!log.contains("kernel ACCEPTED"), "{log}");
    assert!(
        WITNESS_SOURCE.contains("inductive Node (α : Type u) (β : Type v) : Type (max u v) where")
    );
    assert!(WITNESS_SOURCE.contains("| entries (es : Array (Entry α β (Node α β))) : Node α β"));
}

#[test]
fn a_family_nesting_through_a_container_at_an_equal_universe_imports() {
    let (ctx, report) = adapter::import_audited(WITNESS)
        .unwrap_or_else(|v| panic!("Lean accepts every declaration: {v:?}"));
    for family in ["LK_NML.Node", "Lean.PersistentHashMap.Node"] {
        assert!(ctx.is_inductive(&n(family)), "{family} is imported");
        assert_eq!(
            ctx.get_constructors(&n(family)).len(),
            2,
            "{family}: entries and collision"
        );
    }
    for theorem in ["LK_NML.widthEmpty", "LK_NML.nodeEntriesEmpty"] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(theorem))
            .unwrap_or_else(|| panic!("{theorem} audited"));
        assert_eq!(audit.kind, "thm", "{theorem}");
        assert!(audit.axioms.is_empty(), "{theorem}: {:?}", audit.axioms);
    }
}

#[test]
fn the_witness_theorems_emit_leans_statements() {
    bridged(WITNESS, "LK_NML.widthEmpty", "thm", &[]);
    bridged(WITNESS, "LK_NML.nodeEntriesEmpty", "thm", &[]);
    bridged(WITNESS, "LK_NML.Node.width", "def", &[]);
}

/// The witness with the families' shared header `Π α β. Type (max u v)` rewritten to
/// `Π α β. Type u`: the container instance `Entry α β (Node α β)` then lives in `Type (max u v)`,
/// above the family's universe — the shape Lean's kernel refuses for `BadT`.
fn node_below_its_container() -> String {
    let mut rows: Vec<Value> = WITNESS
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let header = rows
        .iter()
        .find_map(|r| {
            let t = &r.get("inductive")?["types"][0];
            (t["numNested"].as_u64()? > 0).then(|| t["type"].as_u64())?
        })
        .expect("the first nested family's header");
    let expr = |rows: &[Value], id: u64| {
        rows.iter()
            .position(|r| r["ie"].as_u64() == Some(id))
            .expect("expression")
    };
    let outer = expr(&rows, header);
    let alpha_domain = rows[outer]["forallE"]["type"].clone();
    let inner = expr(&rows, rows[outer]["forallE"]["body"].as_u64().unwrap());
    rows[inner]["forallE"]["body"] = alpha_domain;
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn a_family_below_its_container_instance_is_rejected_as_lean_rejects_it() {
    match adapter::check_str(&node_below_its_container()) {
        Err(Verdict::Reject(reason)) => assert_ne!(reason.code(), DiagnosticCode::MalformedExport, "{reason:?}"),
        other => panic!("Lean's kernel rejects a nested family below its container instance's universe; so must the reader: {other:?}"),
    }
}
