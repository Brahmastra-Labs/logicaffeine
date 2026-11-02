use super::*;

/// A property is equivalent to itself — the load-bearing identity, RUP-certified.
#[test]
fn sva_self_equivalence_is_certified() {
    let a = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
    let b = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
    let r = prove_bounded_equivalence(&a.expr, &b.expr, 3).unwrap();
    assert!(r.equivalent, "self-equivalence must hold");
    assert!(r.counterexample.is_none());
}

/// The killer case: De Morgan duals are structurally DIFFERENT but semantically EQUAL.
/// Structural equality (`check_bounded_equivalence`) cannot see it; our certified SAT
/// prover can. This is the whole point of semantic equivalence.
#[test]
fn de_morgan_is_semantically_equivalent() {
    let a = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
    let b = translate_sva_to_bounded("!grant_a || !grant_b", 3).unwrap();
    assert!(
        !check_bounded_equivalence(&a.expr, &b.expr, 3).equivalent,
        "structural equality should NOT see De Morgan duals as equal"
    );
    let r = prove_bounded_equivalence(&a.expr, &b.expr, 3).unwrap();
    assert!(
        r.equivalent,
        "De Morgan duals must be semantically equivalent"
    );
}

/// End-to-end: synthesizing SVA from an English spec must PRESERVE meaning — the
/// synthesized SVA is certified equivalent to the spec's own FOL, by our prover.
#[test]
fn synthesized_sva_is_equivalent_to_its_spec() {
    let spec = "Always, if request is high, then acknowledge is high.";
    let synth = crate::codegen_sva::fol_to_sva::synthesize_sva_from_spec(spec, "clk").unwrap();
    let r = prove_spec_sva_equivalence(spec, &synth.body, 3).unwrap();
    assert!(r.equivalent, "synthesis must preserve meaning; got {:?}", r);
}

/// Distinct properties differ, and we recover a concrete counterexample trace.
#[test]
fn distinct_properties_differ_with_counterexample() {
    let a = translate_sva_to_bounded("!(grant_a && grant_b)", 2).unwrap();
    let b = translate_sva_to_bounded("grant_a |-> grant_b", 2).unwrap();
    let r = prove_bounded_equivalence(&a.expr, &b.expr, 2).unwrap();
    assert!(
        !r.equivalent,
        "a mutex and an implication are not equivalent"
    );
    let ce = r.counterexample.expect("a counterexample trace");
    assert!(!ce.is_empty(), "counterexample must bind some signal@t");
}

#[test]
fn implication_spec_has_a_reachable_trigger() {
    // The antecedent (`request is high`) is over a free signal, so it is reachable —
    // the property is non-vacuous.
    let r = check_spec_vacuity("Always, if request is high, then acknowledge is high.", 3).unwrap();
    assert_eq!(r, VacuityReport::NonVacuous);
}

#[test]
fn kg_summary_extracts_signals_and_relations() {
    let kg = kg_summary("Always, if request is high, then acknowledge is high.")
        .expect("extracts a knowledge graph");
    assert!(!kg.nodes.is_empty(), "the spec mentions signals: {kg:?}");
    // Every link must reference valid node indices.
    for l in &kg.links {
        assert!(
            l.from < kg.nodes.len() && l.to < kg.nodes.len(),
            "dangling link {l:?}"
        );
        assert!(!l.relation.is_empty());
    }
    // The named signals should appear (proper nouns are capitalized in the KG).
    let names: Vec<&str> = kg.nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.eq_ignore_ascii_case("request")),
        "expected a Request signal among {names:?}"
    );
    assert!(
        names.iter().any(|n| n.eq_ignore_ascii_case("acknowledge")),
        "expected an Acknowledge signal among {names:?}"
    );
}

#[test]
fn waveform_groups_boolean_signals_over_timesteps() {
    let ce = vec![
        ("req@0".to_string(), "0".to_string()),
        ("req@1".to_string(), "1".to_string()),
        ("ack@1".to_string(), "0".to_string()),
    ];
    let wf = counterexample_waveform(&ce);
    assert_eq!(wf.timesteps, 2);
    assert_eq!(wf.signals.len(), 2);
    let ack = wf.signals.iter().find(|s| s.name == "ack").unwrap();
    assert_eq!(ack.width, 1);
    assert_eq!(ack.values, vec![None, Some(0)]);
    let req = wf.signals.iter().find(|s| s.name == "req").unwrap();
    assert_eq!(req.values, vec![Some(0), Some(1)]);
}

#[test]
fn waveform_reconstructs_multibit_register_values() {
    // cnt is 2 bits: at t0 = 0b01 = 1, at t1 = 0b10 = 2.
    let ce = vec![
        ("cnt@0#0".to_string(), "1".to_string()),
        ("cnt@0#1".to_string(), "0".to_string()),
        ("cnt@1#0".to_string(), "0".to_string()),
        ("cnt@1#1".to_string(), "1".to_string()),
    ];
    let wf = counterexample_waveform(&ce);
    let cnt = wf.signals.iter().find(|s| s.name == "cnt").unwrap();
    assert_eq!(cnt.width, 2);
    assert_eq!(cnt.values, vec![Some(1), Some(2)]);
}

#[test]
fn waveform_skips_malformed_keys() {
    let ce = vec![("nonsense".to_string(), "1".to_string())];
    assert_eq!(counterexample_waveform(&ce), Waveform::default());
}

#[test]
fn first_antecedent_extracts_the_trigger() {
    use crate::codegen_sva::sva_to_verify::BoundedExpr;
    let v = |s: &str| BoundedExpr::Var(s.to_string());
    let bx = |e: BoundedExpr| Box::new(e);
    // ⋀ over timesteps of (req@t → ack@t): the first antecedent is req@0.
    let prop = BoundedExpr::And(
        bx(BoundedExpr::Implies(bx(v("req@0")), bx(v("ack@0")))),
        bx(BoundedExpr::Implies(bx(v("req@1")), bx(v("ack@1")))),
    );
    assert_eq!(first_antecedent(&prop), Some(&v("req@0")));
    // A pure safety property (no implication) has no trigger.
    let safety = BoundedExpr::Not(bx(BoundedExpr::And(bx(v("a@0")), bx(v("b@0")))));
    assert_eq!(first_antecedent(&safety), None);
}
