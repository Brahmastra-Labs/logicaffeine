//! Library-foundation gates for reconstructing Lean's native decision as a new checked proof.
//! The positive export is the unchanged pinned original. Mutants are test-local JSON strings.
//! No package policy or original Lean oracle is changed by these controls.
#[path = "harvest_support/checker.rs"]
mod checker;

use logicaffeine_kernel::interface::lean::{self as lean, native_decide, reference};
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term};
use serde_json::{json, Value};
use std::path::PathBuf;

const TARGET: &str = "T08_014.viaNative";
const STEM: &str = "T08-014.viaNative";
const EXPORT_SHA: &str = "bb2f24bc2e131b2c40301a8130ec551662d2deae0bd19fdcf98fb988a2810273";

fn key() -> String {
    reference::kernel_name_of_components(&["T08_014", "viaNative"])
}
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414/native_decide")
}
fn original() -> String {
    let (export, provenance) = checker::fixture_in(&fixture_dir(), STEM);
    assert_eq!(lean::package::sha256_hex(export.as_bytes()), EXPORT_SHA);
    assert_eq!(provenance["declarations"], json!([TARGET]));
    assert_eq!(
        provenance["lean_print_axioms"],
        json!(["'T08_014.viaNative' depends on axioms: [Lean.ofReduceBool]"])
    );
    let oracle: Value = serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lean_conformance/lean_ground_truth/T08-014.json"),
        )
        .expect("original Lean oracle"),
    )
    .expect("oracle JSON");
    assert_eq!(oracle["lean"], "4.14.0");
    assert_eq!(oracle["compiles"], true);
    let target = oracle["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == TARGET)
        .expect("original target in oracle");
    assert_eq!(target["kind"], "theorem");
    assert_eq!(target["axioms"], json!([native_decide::ELIMINATED_AXIOM]));
    export
}
fn imported(export: &str) -> Result<(lean::ImportedContext, lean::ImportReport), lean::Verdict> {
    lean::import_with(
        export,
        lean::ImportRequest {
            audit: true,
            retain: lean::RetainSource::Named([key()].into_iter().collect()),
            lean_binder_names: true,
        },
    )
}
fn one_leaf(export: &str) -> native_decide::NativeDecision {
    let decisions = native_decide::NativeDecisions::read(export);
    let leaves: Vec<_> = decisions.decisions().cloned().collect();
    assert_eq!(
        leaves.len(),
        1,
        "the authentic fixture has exactly one native decision"
    );
    assert_eq!(leaves[0].decision, "T08_014.viaNative._nativeDecide_1");
    assert!(decisions.closure_of(&key()).eliminates());
    leaves.into_iter().next().unwrap()
}
fn statement(export: &str) -> Term {
    let reference = reference::Reference::read(export).expect("independent reference tables");
    let declaration = reference
        .declaration(&key())
        .expect("independent original statement");
    assert_eq!(declaration.kind, reference::ReferenceKind::Theorem);
    assert_eq!(declaration.lean_name, TARGET);
    declaration.ty
}
fn against(proof: &Term, expected: &Term) -> Term {
    // Each checker must convert the proof's type to expected inside its own typing judgment.
    Term::App(
        Box::new(Term::Lambda {
            param: "__native_expected".into(),
            param_type: Box::new(expected.clone()),
            body: Box::new(Term::Var("__native_expected".into())),
        }),
        Box::new(proof.clone()),
    )
}
fn both_accept(ctx: &Context, term: &Term) {
    infer_type(ctx, term).unwrap_or_else(|e| panic!("primary checker: {e:?}"));
    recheck(ctx, term).unwrap_or_else(|e| panic!("independent checker: {e:?}"));
}
fn invalid_computation(export: &str) {
    match imported(export) {
        Ok(_) => panic!("a wrong native proof must not produce a checked context"),
        Err(error) => assert_eq!(error.reason().code(), lean::DiagnosticCode::InvalidProof,
            "a bounded false computation must be rejected, not treated as unsupported or exhausted: {error:?}"),
    }
}
fn rows(export: &str) -> Vec<Value> {
    export
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn encode(rows: &[Value]) -> String {
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row).unwrap());
        out.push('\n');
    }
    out
}
fn expression(rows: &[Value], id: usize) -> &Value {
    rows.iter()
        .find(|row| row["ie"].as_u64() == Some(id as u64))
        .expect("existing expression")
}
fn expression_mut(rows: &mut [Value], id: usize) -> &mut Value {
    rows.iter_mut()
        .find(|row| row["ie"].as_u64() == Some(id as u64))
        .expect("existing expression")
}
fn changed_natural(export: &str, from: &str, to: &str) -> String {
    let mut rows = rows(export);
    let matches: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| (row["natVal"].as_str() == Some(from)).then_some(i))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "mutate one shared literal record, not an arbitrary text occurrence"
    );
    rows[matches[0]]["natVal"] = json!(to);
    encode(&rows)
}
fn bool_false_expression(rows: &[Value]) -> u64 {
    let bool_name = rows
        .iter()
        .find(|row| {
            row["str"]["pre"].as_u64() == Some(0) && row["str"]["str"].as_str() == Some("Bool")
        })
        .and_then(|row| row["in"].as_u64())
        .expect("the authentic Bool namespace");
    let false_name = rows
        .iter()
        .find(|row| {
            row["str"]["pre"].as_u64() == Some(bool_name)
                && row["str"]["str"].as_str() == Some("false")
        })
        .and_then(|row| row["in"].as_u64())
        .expect("the authentic Bool.false constructor name");
    rows.iter()
        .find(|row| {
            row["const"]["name"].as_u64() == Some(false_name) && row["const"]["us"] == json!([])
        })
        .and_then(|row| row["ie"].as_u64())
        .expect("existing Bool.false expression")
}

#[test]
fn original_native_decision_has_a_new_axiom_free_proof_of_its_exact_original_statement() {
    let export = original();
    one_leaf(&export);
    let expected = statement(&export);
    let (ctx, report) =
        imported(&export).unwrap_or_else(|e| panic!("authentic reconstruction: {e:?}"));
    let (retained_type, proof) = ctx
        .retained(&key())
        .expect("checked replacement proof retained");
    assert!(defeq_for_test(&ctx, retained_type, &expected));
    both_accept(&ctx, &expected);
    both_accept(&ctx, &against(proof, &expected));
    let assumptions =
        checker::lean_semantics_axioms(&ctx, &key(), "theorem", &expected, Some(proof))
            .expect("replacement proof has no unresolved dependencies");
    assert!(
        assumptions.is_empty(),
        "a replacement proof must not merely substitute another trust escape: {assumptions:?}"
    );
    let audited = report
        .declarations
        .iter()
        .find(|row| row.name == key())
        .expect("target independently audited");
    assert!(audited.axioms.is_empty());
    let recognized = native_decide::NativeDecisions::read(&export);
    assert_eq!(report.native_decisions, recognized.rederivations());
    assert_eq!(report.native_decisions.len(), 1);
    assert_eq!(report.native_decisions[0].lean, TARGET);
    // `original()` verifies the unchanged oracle still says Lean.ofReduceBool.
    assert_eq!(lean::package::sha256_hex(export.as_bytes()), EXPORT_SHA);
}

#[test]
fn consistently_false_native_computation_is_recognized_but_not_trusted() {
    let export = original();
    // The shared 100 literal occurs in the theorem, decidability instance and compiled helper.
    // Thus the recognizer still sees a matched leaf, but 101 * 101 = 10000 is false.
    let mutant = changed_natural(&export, "100", "101");
    one_leaf(&mutant);
    let (ctx, _) = imported(&export).unwrap_or_else(|e| panic!("authentic control: {e:?}"));
    assert!(!defeq_for_test(
        &ctx,
        &statement(&export),
        &statement(&mutant)
    ));
    invalid_computation(&mutant);
}

#[test]
fn reconstructed_proof_cannot_be_reused_at_a_different_well_formed_statement() {
    let export = original();
    let (ctx, _) = imported(&export).unwrap_or_else(|e| panic!("authentic control: {e:?}"));
    let (_, proof) = ctx.retained(&key()).expect("replacement proof");
    let original_statement = statement(&export);
    let altered = statement(&changed_natural(&export, "10000", "9999"));
    both_accept(&ctx, &original_statement);
    both_accept(&ctx, &altered);
    both_accept(&ctx, &against(proof, &original_statement));
    assert!(!defeq_for_test(&ctx, &original_statement, &altered));
    let wrong = against(proof, &altered);
    assert!(
        infer_type(&ctx, &wrong).is_err(),
        "primary checker accepted 100*100=9999"
    );
    assert!(
        recheck(&ctx, &wrong).is_err(),
        "independent checker accepted 100*100=9999"
    );
}

#[test]
fn missing_reflection_premise_is_not_an_eliminated_native_step() {
    let export = original();
    let leaf = one_leaf(&export);
    let mut rows = rows(&export);
    let without_evaluation = expression(&rows, leaf.appeal)["app"]["fn"].clone();
    // Supply Lean.ofReduceBool d true without its required equality argument.
    expression_mut(&mut rows, leaf.leaf)["app"]["arg"] = without_evaluation;
    let mutant = encode(&rows);
    let decisions = native_decide::NativeDecisions::read(&mutant);
    assert!(decisions.is_empty());
    let closure = decisions.closure_of(&key());
    assert!(closure.escape_remains);
    assert!(!closure.eliminates());
    assert!(closure.information_changes().is_empty());
    invalid_computation(&mutant);
}

#[test]
fn reflection_at_false_is_not_an_eliminated_native_step() {
    let export = original();
    let leaf = one_leaf(&export);
    let mut rows = rows(&export);
    let false_expression = bool_false_expression(&rows);
    // Keep Eq.refl Bool, but change its argument: refl false cannot justify the required = true.
    expression_mut(&mut rows, leaf.evaluation)["app"]["arg"] = json!(false_expression);
    let mutant = encode(&rows);
    let decisions = native_decide::NativeDecisions::read(&mutant);
    assert!(decisions.is_empty());
    assert!(decisions.closure_of(&key()).escape_remains);
    invalid_computation(&mutant);
}

#[test]
fn disclosure_keeps_original_lean_assumptions_distinct_from_checked_replacement_assumptions() {
    let export = original();
    let decisions = native_decide::NativeDecisions::read(&export);
    let closure = decisions.closure_of(&key());
    assert!(closure.eliminates());
    let original_axioms = vec![native_decide::ELIMINATED_AXIOM.to_string()];
    let unchanged = original_axioms.clone();
    assert_eq!(
        closure.lean_after_rederivation(&original_axioms).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        original_axioms, unchanged,
        "never rewrite Lean's original oracle evidence"
    );
    assert!(
        closure.lean_after_rederivation(&[]).is_err(),
        "an omitted original escape must be reported"
    );
    let with_other_axiom = vec![
        "propext".to_string(),
        native_decide::ELIMINATED_AXIOM.to_string(),
    ];
    assert_eq!(
        closure.lean_after_rederivation(&with_other_axiom).unwrap(),
        vec!["propext".to_string()],
        "only the specifically reconstructed escape can be removed from the expected set"
    );
    let disclosure = closure.information_changes();
    assert_eq!(disclosure.len(), 1);
    assert_eq!(disclosure[0].0, native_decide::INFORMATION_CATEGORY);
    assert!(disclosure[0].1.contains(TARGET));
    assert!(disclosure[0].1.contains(native_decide::ELIMINATED_AXIOM));
    assert_eq!(
        closure.rederivations[0].to_json()["method"],
        "kernel_evaluation"
    );
    closure
        .disclosed_by(disclosure.iter().map(|(c, d)| (c.as_str(), d.as_str())))
        .unwrap();
    assert!(closure
        .disclosed_by(std::iter::empty::<(&str, &str)>())
        .is_err());
    assert!(closure
        .disclosed_by([(native_decide::INFORMATION_CATEGORY, "no proof changed")])
        .is_err());
    assert!(
        closure
            .disclosed_by(
                disclosure
                    .iter()
                    .chain(disclosure.iter())
                    .map(|(c, d)| (c.as_str(), d.as_str()))
            )
            .is_err(),
        "duplicate disclosures are not exact disclosure"
    );
}

#[test]
fn a_general_compiler_bridge_is_not_reclassified_as_a_native_decision_proof() {
    let (export, _) = checker::fixture_in(&fixture_dir(), "T06-014.bridgeShape");
    let target = reference::kernel_name_of_components(&["T06_014", "bridgeShape"]);
    let decisions = native_decide::NativeDecisions::read(&export);
    assert!(decisions.is_empty());
    let closure = decisions.closure_of(&target);
    assert!(closure.escape_remains);
    assert!(!closure.eliminates());
    let original = vec![
        "Lean.trustCompiler".to_string(),
        "Lean.ofReduceBool".to_string(),
    ];
    assert_eq!(
        closure.lean_after_rederivation(&original).unwrap(),
        original
    );
    assert!(closure.information_changes().is_empty());
}
