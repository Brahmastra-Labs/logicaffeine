//! An imported family and an imported recursor state exactly what Lean states.
//!
//! The reader checks every family and every recursor it imports, and what it registers is the
//! declaration's statement: the source the emitter writes and the type R8 compares with an
//! independent reference translation of the Lean export. Two statements used to be registered
//! in a form Lean never wrote:
//!
//! * a family's arity, normalized, so `outParam` / `semiOutParam` vanished from the parameter
//!   domains of harvest T07-006's four classes (`Decode`, `HPlus`, `Mem`, `Rel`). Erasing the
//!   wrapper is the trap T07-006 names: it typechecks, and it is a different class;
//! * the derived recursor's own type, so harvest T11-012.h2's `VTree.rec_1` stated its induction
//!   hypothesis at the literal `2` where Lean's `rec_1` says `OfNat.ofNat Nat 2 (instOfNatNat 2)`.
//!
//! Both are definitionally equal to Lean's statements and both are still checked against the
//! kernel's own derivation by exact conversion; what is registered is Lean's statement.
use logicaffeine_kernel::interface::lean::{self as adapter, emit, reference, Verdict};
use logicaffeine_kernel::{Context, Term};
use serde_json::Value;

const T07_006: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/harvest/T07-006.ndjson");
const T11_012_H2: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/harvest/T11-012.h2.ndjson");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

/// The registered statement of `key`: its polymorphic header, or its monomorphic type.
fn statement(ctx: &Context, key: &str) -> Term {
    ctx.poly_declaration(key)
        .map(|(_, ty)| ty.clone())
        .or_else(|| ctx.get_global(key).cloned())
        .unwrap_or_else(|| panic!("{key} is registered"))
}

fn mentions(t: &Term, key: &str) -> bool {
    match t {
        Term::Global(g) | Term::Const { name: g, .. } => g == key,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions(param_type, key) || mentions(body_type, key),
        Term::Lambda {
            param_type, body, ..
        } => mentions(param_type, key) || mentions(body, key),
        Term::App(f, a) => mentions(f, key) || mentions(a, key),
        Term::Let {
            ty, value, body, ..
        } => mentions(ty, key) || mentions(value, key) || mentions(body, key),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions(discriminant, key)
                || mentions(motive, key)
                || cases.iter().any(|c| mentions(c, key))
        }
        Term::Fix { body, .. } => mentions(body, key),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| mentions(b, key)),
        _ => false,
    }
}

/// R8 for `target`, emitted from `export`, bound to the reference translation of `reference_export`.
fn r8(export: &str, reference_export: &str, target: &str) -> Result<(), String> {
    let intake = adapter::intake::read(export, &adapter::intake::STAGE_A)
        .map_err(|e| format!("intake: {e:?}"))?;
    let closure = intake
        .closure(target)
        .map_err(|e| format!("closure: {e:?}"))?;
    let (imported, _) = adapter::import_retaining_closure(export, target, &closure)
        .map_err(|e| format!("import: {e:?}"))?;
    let doc = emit::emit_document(&imported, target).map_err(|e| format!("emission: {e:?}"))?;
    let original = reference::reference_declaration(reference_export, target)
        .map_err(|e| format!("reference: {e:?}"))?;
    emit::check_statement(&doc, &original).map_err(|f| format!("{f}"))
}

const CLASSES: [(&str, &str); 4] = [
    ("T07_006.Decode", "outParam"),
    ("T07_006.HPlus", "outParam"),
    ("T07_006.Mem", "outParam"),
    ("T07_006.Rel", "semiOutParam"),
];

#[test]
fn a_class_family_is_registered_with_its_output_parameters_as_lean_declares_them() {
    let (ctx, _) =
        adapter::import_audited(T07_006).unwrap_or_else(|v| panic!("Lean accepts T07-006: {v:?}"));
    for (class, wrapper) in CLASSES {
        let arity = statement(&ctx, &n(class));
        assert!(
            mentions(&arity, &n(wrapper)),
            "{class}: its arity keeps `{wrapper}`: {arity}"
        );
        let lean = reference::reference_declaration(T07_006, &n(class))
            .expect("reference")
            .ty;
        assert!(
            mentions(&lean, &n(wrapper)),
            "{class}: Lean's own statement has `{wrapper}`"
        );
    }
}

#[test]
fn every_t07_006_class_family_emits_leans_statement() {
    for (class, _) in CLASSES {
        r8(T07_006, T07_006, &n(class)).unwrap_or_else(|f| panic!("{class}: {f}"));
        r8(T07_006, T07_006, &n(&format!("{class}.mk")))
            .unwrap_or_else(|f| panic!("{class}.mk: {f}"));
    }
}

#[test]
fn a_nested_auxiliary_recursor_states_its_hypothesis_at_leans_index() {
    let (ctx, _) = adapter::import_audited(T11_012_H2)
        .unwrap_or_else(|v| panic!("Lean accepts T11-012.h2: {v:?}"));
    let rec_1 = statement(&ctx, &n("T11_012.VTree.rec_1"));
    assert!(
        mentions(&rec_1, &n("OfNat.ofNat")),
        "the index is `OfNat.ofNat Nat 2 _`, as Lean writes it: {rec_1}"
    );
    r8(T11_012_H2, T11_012_H2, &n("T11_012.VTree.rec_1"))
        .unwrap_or_else(|f| panic!("VTree.rec_1: {f}"));
    r8(T11_012_H2, T11_012_H2, &n("T11_012.VTree.rec"))
        .unwrap_or_else(|f| panic!("VTree.rec: {f}"));
}

// -------------------------------------------------------------------------------------------------
// Controls. The statement carried is the export's, whatever it is: an arity stripped of its
// `outParam` is carried stripped, and R8 against Lean's original refuses it. A recursor type that
// is not the derivation's is refused before anything is registered.
// -------------------------------------------------------------------------------------------------

fn rows(export: &str) -> Vec<Value> {
    export
        .lines()
        .map(|l| serde_json::from_str(l).expect("ndjson row"))
        .collect()
}

fn write(rows: &[Value]) -> String {
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

/// T07-006 with `Decode`'s second binder `(β : outParam Type)` rewritten as `(β : Type)`: the
/// expression under `Decode`'s arity is used by that arity alone, so only the family's statement
/// changes; its constructor still writes `{β : outParam Type}`.
fn decode_without_out_param() -> String {
    let mut rows = rows(T07_006);
    let decode = rows
        .iter()
        .position(|r| r.get("inductive").is_some())
        .expect("Decode is the export's first family");
    let arity = rows[decode]["inductive"]["types"][0]["type"]
        .as_u64()
        .expect("Decode's arity");
    let expr = |rows: &[Value], id: u64| {
        rows.iter()
            .position(|r| r["ie"].as_u64() == Some(id))
            .expect("expression")
    };
    let outer = expr(&rows, arity);
    let inner = rows[outer]["forallE"]["body"]
        .as_u64()
        .expect("Decode's arity binds α, then β");
    let at = expr(&rows, inner);
    let wrapped = rows[at]["forallE"]["type"].as_u64().expect("β's domain");
    let bare = rows[expr(&rows, wrapped)]["app"]["arg"]
        .as_u64()
        .expect("`outParam Type` applies outParam to Type");
    rows[at]["forallE"]["type"] = Value::from(bare);
    write(&rows)
}

#[test]
fn an_arity_the_export_states_without_out_param_is_carried_as_stated_and_r8_refuses_it() {
    let stripped = decode_without_out_param();
    let (ctx, _) = adapter::import_audited(&stripped)
        .unwrap_or_else(|v| panic!("still a valid declaration: {v:?}"));
    let arity = statement(&ctx, &n("T07_006.Decode"));
    assert!(
        !mentions(&arity, &n("outParam")),
        "the stripped export's statement is carried stripped: {arity}"
    );
    assert!(
        mentions(&statement(&ctx, &n("T07_006.Decode.mk")), &n("outParam")),
        "its constructor still writes it"
    );
    let refused = r8(&stripped, T07_006, &n("T07_006.Decode"))
        .expect_err("the stripped statement is not Lean's");
    assert!(
        refused.contains("does not state the reference translation"),
        "{refused}"
    );
    r8(&stripped, &stripped, &n("T07_006.Decode"))
        .unwrap_or_else(|f| panic!("and it is its own export's: {f}"));
}

#[test]
fn a_recursor_type_that_is_not_the_derivations_is_refused() {
    let mut rows = rows(T11_012_H2);
    let (at, recs) = rows
        .iter()
        .enumerate()
        .find_map(|(i, r)| {
            let recs = r.get("inductive")?["recs"].as_array()?;
            (recs.len() == 2 && recs.iter().any(|x| x["numIndices"] == 1))
                .then(|| (i, recs.clone()))
        })
        .expect("VTree's block: rec_1 (one index) and rec");
    let aux = recs.iter().position(|x| x["numIndices"] == 1).unwrap();
    rows[at]["inductive"]["recs"][aux]["type"] = recs[1 - aux]["type"].clone();
    match adapter::check_str(&write(&rows)) {
        Err(Verdict::Reject(_)) => {}
        other => panic!("VTree.rec_1 stated at VTree.rec's type must be rejected: {other:?}"),
    }
}
