//! A conversion that reduction could not finish is a RESOURCE LIMIT, never a wrong rejection — and a
//! conversion the kernel CAN finish is decided.
//!
//! `LF_NB.charBound : (1114112 : Nat) < 4294967296 := by decide` is an original Lean 4.14.0 witness
//! (`fixtures/lean_semantics/LF_NatBle.lean`) of the bound `isValidChar_UInt32` decides (42 open
//! roots). This kernel used to reduce Lean's imported `Nat.ble` in unary and run out of budget; it now
//! accelerates `Nat.ble` on literals in both kernels (`lean_scale_nat_ble`), so the theorem is
//! ACCEPTED, exactly as Lean accepts it.
//!
//! The resource-limit door still needs a witness that is genuinely beyond the budget:
//! `KA_UD.sweepOpen` (`fixtures/lean_semantics/KA_Undecided.lean`, original Lean 4.14.0) checks by
//! `rfl` only after 1 124 250 Boolean flips — at least 2 248 500 iota steps against a budget of 2^20
//! whole-term steps — on an OPEN term, which no closed-data evaluator may shortcut. It must decline
//! with `ResourceLimit`, never be called an invalid proof. And a conversion that is decidably false
//! stays an `InvalidProof` however the budget stands.
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{infer_type, is_subtype, recheck};
use serde_json::{json, Value};

const CHAR_BOUND: &str = include_str!("fixtures/lean_semantics/LF_NatBle.charBound.ndjson");
const CHAR_BOUND_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_NatBle.charBound.provenance.json");
const CHAR_BOUND_SOURCE: &str = include_str!("fixtures/lean_semantics/LF_NatBle.lean");
const SWEEP: &str = include_str!("fixtures/lean_semantics/KA_Undecided.sweepOpen.ndjson");
const SWEEP_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/KA_Undecided.sweepOpen.provenance.json");
const SWEEP_SOURCE: &str = include_str!("fixtures/lean_semantics/KA_Undecided.lean");

/// The provenance of one original export: the pinned toolchain and exporter, the source it came
/// from, and the export this file includes, byte for byte.
fn pinned(
    provenance: &str,
    source: &str,
    source_sha: &str,
    export: &str,
    export_sha: &str,
    roots: &[&str],
) {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(
        p["source"],
        format!("crates/logicaffeine_tests/tests/fixtures/lean_semantics/{source}")
    );
    assert_eq!(p["source_sha256"], source_sha);
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
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["records"], export.lines().count());
    assert_eq!(p["export_header"]["meta"]["lean"]["version"], "4.14.0");
    let declared: Vec<&str> = p["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert_eq!(declared, roots);
}

fn char_bound_pinned() {
    pinned(
        CHAR_BOUND_PROVENANCE,
        "LF_NatBle.lean",
        "21b71393e19e6c4b7bc901d515c27d8fc6877412bede1e8e544beb740d482302",
        CHAR_BOUND,
        "6b1997103bae53dd39486e9b27848c693d641ddd3a63ad6e08f6c33145d5c7bb",
        &["LF_NB.charBound"],
    );
    assert!(
        CHAR_BOUND_SOURCE.contains("theorem charBound : (1114112 : Nat) < 4294967296 := by decide")
    );
}

fn rows(export: &str) -> Vec<Value> {
    export
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn text(rows: &[Value]) -> String {
    rows.iter().map(|r| format!("{r}\n")).collect()
}
fn next_expression(rows: &[Value]) -> u64 {
    rows.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1
}

#[test]
fn the_original_char_bound_is_accepted_exactly() {
    char_bound_pinned();
    adapter::check_str(CHAR_BOUND)
        .unwrap_or_else(|v| panic!("Lean accepts `charBound`, and so must this kernel: {v:?}"));
    let (ctx, report) = adapter::import_audited(CHAR_BOUND).expect("the audited import");
    let name = r#"/s:"LF_NB"/s:"charBound""#;
    let statement = ctx
        .get_global(name)
        .expect("`charBound` is published")
        .clone();
    let proof = ctx
        .get_definition_body(name)
        .expect("a theorem keeps its proof, as Lean's does")
        .clone();
    let main = infer_type(&ctx, &proof).unwrap_or_else(|e| panic!("primary kernel: {e:?}"));
    let independent = recheck(&ctx, &proof).unwrap_or_else(|e| panic!("independent kernel: {e:?}"));
    for inferred in [&main, &independent] {
        assert!(
            is_subtype(&ctx, inferred, &statement) && is_subtype(&ctx, &statement, inferred),
            "both kernels infer exactly the stated bound: {inferred} vs {statement}"
        );
    }
    // Lean 4.14.0: `#print axioms LF_NB.charBound` — "does not depend on any axioms".
    let audit = report
        .declarations
        .iter()
        .find(|d| d.name == "LF_NB.charBound" || d.name == name)
        .unwrap_or_else(|| {
            panic!(
                "the audit lists `charBound`: {:?}",
                report
                    .declarations
                    .iter()
                    .map(|d| &d.name)
                    .collect::<Vec<_>>()
            )
        });
    assert!(
        audit.axioms.is_empty(),
        "`charBound` rests on no axiom, as in Lean: {:?}",
        audit.axioms
    );
}

#[test]
fn the_char_bound_proof_at_the_swapped_bound_is_an_invalid_proof() {
    // `(4294967296 : Nat) < 1114112` is decidably false, and the budget has nothing to do with it:
    // the same proof at that statement is refused as an invalid proof.
    let mut rows = rows(CHAR_BOUND);
    let at = rows
        .iter()
        .rposition(|r| r.get("thm").is_some())
        .expect("the root theorem is the last record");
    let ty = rows[at]["thm"]["type"].as_u64().unwrap();
    let node =
        |rows: &[Value], id: u64| rows.iter().find(|r| r["ie"] == id).unwrap()["app"].clone();
    let lt_lo = node(&rows, ty);
    let lt = node(&rows, lt_lo["fn"].as_u64().unwrap());
    let (lo, hi) = (lt["arg"].clone(), lt_lo["arg"].clone());
    let next = next_expression(&rows);
    let swapped = [
        json!({"ie": next, "app": {"fn": lt["fn"], "arg": hi}}),
        json!({"ie": next + 1, "app": {"fn": next, "arg": lo}}),
    ];
    for (k, row) in swapped.iter().enumerate() {
        rows.insert(at + k, row.clone());
    }
    rows[at + swapped.len()]["thm"]["type"] = json!(next + 1);
    let verdict =
        adapter::check_str(&text(&rows)).expect_err("a proof of `a < b` does not prove `b < a`");
    assert!(matches!(verdict, Verdict::Reject(_)), "{verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{verdict:?}"
    );
}

#[test]
fn an_unfinished_conversion_on_valid_lean_is_a_resource_limit_not_an_invalid_proof() {
    pinned(
        SWEEP_PROVENANCE,
        "KA_Undecided.lean",
        "95dda763e7b4199e4fd8d0c094a000b73aacf083b52c725a69c68f6c31dcb04a",
        SWEEP,
        "948c0f16bfc956d2af44afa4b42a4f46a9c39a01341786c61c9eb2c48dc9fdee",
        &["KA_UD.sweepOpen"],
    );
    assert!(SWEEP_SOURCE
        .contains("theorem sweepOpen (b : Bool) : sweep 1500 (true || b) = true := rfl"));
    let verdict =
        adapter::check_str(SWEEP).expect_err("the conversion is beyond this kernel's budget");
    assert!(
        matches!(verdict, Verdict::Decline(_)),
        "valid Lean is never rejected: {verdict:?}"
    );
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::ResourceLimit,
        "{verdict:?}"
    );
    assert!(
        verdict.reason().message().contains("undecided"),
        "{verdict:?}"
    );
}

#[test]
fn a_decidably_false_conversion_is_still_an_invalid_proof() {
    // The export's first theorem restated at `@Eq.{1} Nat 0 1`: a different proposition, and
    // deciding that needs no reduction budget, so the refusal stays an InvalidProof.
    char_bound_pinned();
    let mut rows = rows(CHAR_BOUND);
    let at = rows
        .iter()
        .position(|r| r.get("thm").is_some())
        .expect("the theorem record");
    let name = |rows: &[Value], s: &str| {
        rows.iter()
            .find(|r| r["str"]["pre"] == 0 && r["str"]["str"] == s)
            .unwrap()["in"]
            .clone()
    };
    let (nat, eq) = (name(&rows, "Nat"), name(&rows, "Eq"));
    let one = rows.iter().find(|r| r["succ"] == 0).expect("level 1")["il"].clone();
    let next = next_expression(&rows);
    let new = [
        json!({"ie": next, "const": {"name": eq, "us": [one]}}),
        json!({"ie": next + 1, "const": {"name": nat, "us": []}}),
        json!({"ie": next + 2, "natVal": "0"}),
        json!({"ie": next + 3, "natVal": "1"}),
        json!({"ie": next + 4, "app": {"fn": next, "arg": next + 1}}),
        json!({"ie": next + 5, "app": {"fn": next + 4, "arg": next + 2}}),
        json!({"ie": next + 6, "app": {"fn": next + 5, "arg": next + 3}}),
    ];
    for (k, row) in new.iter().enumerate() {
        rows.insert(at + k, row.clone());
    }
    rows[at + new.len()]["thm"]["type"] = json!(next + 6);
    let verdict = adapter::check_str(&text(&rows)).expect_err("a proof at the wrong statement");
    assert!(matches!(verdict, Verdict::Reject(_)), "{verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{verdict:?}"
    );
}
