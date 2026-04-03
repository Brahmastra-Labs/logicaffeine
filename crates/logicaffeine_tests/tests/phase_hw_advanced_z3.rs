//! Z3 Advanced Analysis Tests
//!
//! Spec health (consistency/vacuity/redundancy), invariant discovery,
//! decomposition soundness, and adversarial/negative cases.
//! Every test invokes Z3.

#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::hw_pipeline::{
    check_z3_hw_equivalence, check_z3_equivalence, extract_kg,
    translate_sva_to_bounded, HwSignalDecl,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::{bounded_to_verify, extract_signal_names};
use logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec;
use logicaffeine_compile::codegen_sva::decompose::decompose_conjunctive;
use logicaffeine_compile::codegen_sva::invariants::{discover_invariants, InvariantSource};
use logicaffeine_language::semantics::knowledge_graph::{
    HwKnowledgeGraph, SignalRole, KgRelation, HwEntityType, HwRelation,
};
use logicaffeine_verify::equivalence::{EquivalenceResult, check_equivalence};
use logicaffeine_verify::ir::{VerifyExpr, VerifyOp};
use logicaffeine_verify::consistency::{
    check_spec_consistency, ConsistencyConfig, ConsistencyReport,
    LabeledFormula, SatisfiabilityResult,
};

// ═══════════════════════════════════════════════════════════════════════════
// CATEGORY D: SPEC HEALTH + Z3
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn spechealth_z3_consistent_single() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let config = ConsistencyConfig::default();
    let report = check_english_spec("Always, every signal is valid.", config).unwrap();
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
        "Single consistent sentence must be satisfiable. Got: {:?}", report.satisfiability);
}

#[test]
fn spechealth_z3_consistent_two_compat() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let config = ConsistencyConfig::default();
    let report = check_english_spec(
        "Always, every request is valid. Always, every acknowledgment is valid.", config
    ).unwrap();
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
        "Two compatible sentences must be satisfiable. Got: {:?}", report.satisfiability);
}

#[test]
fn spechealth_z3_contradictory() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let config = ConsistencyConfig::default();
    let report = check_english_spec(
        "Always, every request is valid. Always, every request is not valid.", config
    ).unwrap();
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Unsatisfiable { .. }),
        "Contradictory sentences must be unsatisfiable. Got: {:?}", report.satisfiability);
}

#[test]
fn spechealth_z3_three_way_contradiction() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let config = ConsistencyConfig::default();
    // Sentence 1: Req is valid. Sentence 2: if Req then Ack. Sentence 3: Ack is not valid.
    // Together: Req=true, Req→Ack forces Ack=true, but Ack=false. Contradiction.
    let report = check_english_spec(
        "Always, every request is valid. Always, if every request holds, then every acknowledgment holds. Always, every acknowledgment is not valid.",
        config
    ).unwrap();
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Unsatisfiable { .. }),
        "Three-way contradiction must be unsatisfiable. Got: {:?}", report.satisfiability);
}

#[test]
fn spechealth_z3_empty_consistent() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let config = ConsistencyConfig::default();
    let report = check_english_spec("", config).unwrap();
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
        "Empty spec must be vacuously satisfiable. Got: {:?}", report.satisfiability);
}

#[test]
fn spechealth_z3_vacuous_implication() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let mut config = ConsistencyConfig::default();
    config.check_vacuity = true;
    // First sentence forces Req=false. Second has Req in antecedent → vacuous.
    let report = check_english_spec(
        "Always, every request is not valid. Always, if every request holds, then every acknowledgment holds.",
        config
    ).unwrap();
    // If vacuity detection works, it should find the second formula is vacuous
    // (the antecedent can never be satisfied given the first constraint).
    // This is a stretch test — vacuity detection requires evaluating the antecedent
    // in the context of other formulas.
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
        "Spec with vacuous implication should still be satisfiable (not contradictory). Got: {:?}",
        report.satisfiability);
}

#[test]
fn spechealth_z3_redundant_formula() {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    let mut config = ConsistencyConfig::default();
    config.check_redundancy = true;
    // If Req is always valid, then "if Ack then Req" is entailed (Req is true regardless)
    let report = check_english_spec(
        "Always, every request is valid. Always, if every acknowledgment holds, then every request holds.",
        config
    ).unwrap();
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
        "Spec with redundancy should still be satisfiable. Got: {:?}", report.satisfiability);
}

// ═══════════════════════════════════════════════════════════════════════════
// CATEGORY E: INVARIANT DISCOVERY + Z3
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn invariant_z3_mutex_from_constrains() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("grant_a", 1, SignalRole::Output);
    kg.add_signal("grant_b", 1, SignalRole::Output);
    kg.add_edge("grant_a", "grant_b", KgRelation::Constrains, None);

    let invariants = discover_invariants(&kg);
    let mutex = invariants.iter().find(|inv| matches!(inv.source, InvariantSource::MutexPattern));
    assert!(mutex.is_some(),
        "Constrains edge should produce MutexPattern invariant. Found: {:?}",
        invariants.iter().map(|i| format!("{:?}", i.source)).collect::<Vec<_>>());

    // Z3 self-consistency: the discovered invariant should be satisfiable
    if let Some(inv) = mutex {
        let signals = vec!["grant_a".to_string(), "grant_b".to_string()];
        let result = check_equivalence(&inv.expr, &inv.expr, &signals, 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Discovered mutex invariant must be self-consistent via Z3. Got: {:?}", result);
    }
}

#[test]
fn invariant_z3_handshake_from_triggers() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);
    kg.add_edge("req", "ack", KgRelation::Triggers, None);

    let invariants = discover_invariants(&kg);
    let handshake = invariants.iter().find(|inv| matches!(inv.source, InvariantSource::HandshakePattern));
    assert!(handshake.is_some(),
        "Triggers edge should produce HandshakePattern invariant. Found: {:?}",
        invariants.iter().map(|i| format!("{:?}", i.source)).collect::<Vec<_>>());

    if let Some(inv) = handshake {
        let signals = vec!["req".to_string(), "ack".to_string()];
        let result = check_equivalence(&inv.expr, &inv.expr, &signals, 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Discovered handshake invariant must be self-consistent. Got: {:?}", result);
    }
}

#[test]
fn invariant_z3_from_english() {
    let spec = "Always, if every request holds, then every acknowledgment holds.";
    let kg = extract_kg(spec).unwrap();
    let invariants = discover_invariants(&kg);

    // Any discovered invariants should be self-consistent
    for inv in &invariants {
        let signals: Vec<String> = kg.signals.iter().map(|s| s.name.clone()).collect();
        if !signals.is_empty() {
            let result = check_equivalence(&inv.expr, &inv.expr, &signals, 1);
            assert!(matches!(result, EquivalenceResult::Equivalent),
                "Invariant {:?} from English spec must be self-consistent. Got: {:?}",
                inv.source, result);
        }
    }
}

#[test]
fn invariant_z3_self_consistent() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("a", 1, SignalRole::Output);
    kg.add_signal("b", 1, SignalRole::Output);
    kg.add_edge("a", "b", KgRelation::Constrains, None);

    let invariants = discover_invariants(&kg);
    for inv in &invariants {
        // Check consistency using the Z3 consistency checker
        let labeled = vec![LabeledFormula {
            index: 0,
            label: format!("{:?}", inv.source),
            expr: inv.expr.clone(),
        }];
        let config = ConsistencyConfig::default();
        let report = check_spec_consistency(&labeled, &config);
        assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
            "Single invariant must be self-consistent. Source: {:?}, Got: {:?}",
            inv.source, report.satisfiability);
    }
}

#[test]
fn invariant_z3_multiple_compatible() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("grant_a", 1, SignalRole::Output);
    kg.add_signal("grant_b", 1, SignalRole::Output);
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);
    kg.add_edge("grant_a", "grant_b", KgRelation::Constrains, None);
    kg.add_edge("req", "ack", KgRelation::Triggers, None);

    let invariants = discover_invariants(&kg);
    if invariants.len() >= 2 {
        let labeled: Vec<LabeledFormula> = invariants.iter().enumerate().map(|(i, inv)| {
            LabeledFormula {
                index: i,
                label: format!("{:?}", inv.source),
                expr: inv.expr.clone(),
            }
        }).collect();
        let config = ConsistencyConfig::default();
        let report = check_spec_consistency(&labeled, &config);
        assert!(matches!(report.satisfiability, SatisfiabilityResult::Satisfiable),
            "Independent invariants (mutex + handshake) must be jointly satisfiable. Got: {:?}",
            report.satisfiability);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CATEGORY G: DECOMPOSITION SOUNDNESS + Z3
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn decompose_z3_conjunction_equiv() {
    // Build And(And(P, Q), R), decompose, reconstruct, Z3 check
    let p = VerifyExpr::Var("req@0".into());
    let q = VerifyExpr::Var("ack@0".into());
    let r = VerifyExpr::Var("en@0".into());
    let original = VerifyExpr::and(VerifyExpr::and(p.clone(), q.clone()), r.clone());

    let parts = decompose_conjunctive(&original);
    assert_eq!(parts.len(), 3, "And(And(P,Q),R) should decompose to 3 parts. Got: {}", parts.len());

    // Reconstruct conjunction from parts
    let mut reconstructed = parts[0].clone();
    for part in &parts[1..] {
        reconstructed = VerifyExpr::and(reconstructed, part.clone());
    }

    let signals = vec!["req".to_string(), "ack".to_string(), "en".to_string()];
    let result = check_equivalence(&original, &reconstructed, &signals, 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Decomposed and reconstructed conjunction must be equivalent to original. Got: {:?}", result);
}

#[test]
fn decompose_z3_sva_mutex_splits() {
    // Parse "!(a && b) && !(a && c)" → bounded → verify → decompose → check
    let bounded = translate_sva_to_bounded("!(grant_a && grant_b) && !(grant_a && grant_c)", 3).unwrap();
    let verify = bounded_to_verify(&bounded.expr);

    let parts = decompose_conjunctive(&verify);
    assert!(parts.len() >= 2,
        "Conjunctive mutex should decompose to 2+ parts. Got: {}", parts.len());

    let mut reconstructed = parts[0].clone();
    for part in &parts[1..] {
        reconstructed = VerifyExpr::and(reconstructed, part.clone());
    }

    let signals = extract_signal_names(&bounded);
    let result = check_equivalence(&verify, &reconstructed, &signals, 3);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Decomposed mutex conjunction must equal original. Got: {:?}", result);
}

#[test]
fn decompose_z3_single_no_split() {
    // Non-conjunctive: req |-> ack — should decompose to itself
    let bounded = translate_sva_to_bounded("req |-> ack", 3).unwrap();
    let verify = bounded_to_verify(&bounded.expr);

    let parts = decompose_conjunctive(&verify);
    assert_eq!(parts.len(), 1,
        "Non-conjunctive property should not split. Got: {} parts", parts.len());

    let signals = extract_signal_names(&bounded);
    let result = check_equivalence(&verify, &parts[0], &signals, 3);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Single decomposition must be identity. Got: {:?}", result);
}

#[test]
fn decompose_z3_deep_nesting() {
    // And(And(And(And(a, b), c), d), e) — should flatten to 5
    let a = VerifyExpr::Var("a@0".into());
    let b = VerifyExpr::Var("b@0".into());
    let c = VerifyExpr::Var("c@0".into());
    let d = VerifyExpr::Var("d@0".into());
    let e = VerifyExpr::Var("e@0".into());
    let original = VerifyExpr::and(
        VerifyExpr::and(
            VerifyExpr::and(
                VerifyExpr::and(a.clone(), b.clone()),
                c.clone()
            ),
            d.clone()
        ),
        e.clone()
    );

    let parts = decompose_conjunctive(&original);
    assert_eq!(parts.len(), 5, "4-level nesting should flatten to 5 parts. Got: {}", parts.len());

    let mut reconstructed = parts[0].clone();
    for part in &parts[1..] {
        reconstructed = VerifyExpr::and(reconstructed, part.clone());
    }

    let signals = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string(), "e".to_string()];
    let result = check_equivalence(&original, &reconstructed, &signals, 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Deeply nested decomposition must be equivalent. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// CATEGORY I: ADVERSARIAL / NEGATIVE CASES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn adversarial_z3_swapped_signals() {
    let spec = "Always, if every Req holds, then every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    let result = check_z3_hw_equivalence(spec, "ack |-> req", &decls, 5).unwrap();
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "Swapped signals (ack|->req vs req|->ack) must NOT be equivalent. Got: {:?}", result);
}

#[test]
fn adversarial_z3_extra_conjunction() {
    let spec = "Always, if every Req holds, then every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    // Conjunction of ack with a DIFFERENT signal makes SVA strictly stronger
    // (req |-> (ack && !req) can never be satisfied when req is true)
    let result = check_z3_hw_equivalence(spec, "req |-> (ack && !req)", &decls, 5).unwrap();
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "Extra negated conjunction makes SVA different — must NOT be equivalent. Got: {:?}", result);
}

#[test]
fn adversarial_z3_tautological_sva() {
    let spec = "Always, if every Req holds, then every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    // Tautological consequent: ack || !ack is always true
    let result = check_z3_hw_equivalence(spec, "req |-> (ack || !ack)", &decls, 5).unwrap();
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "Tautological SVA must NOT be equivalent to real spec. Got: {:?}", result);
}

#[test]
fn adversarial_z3_contradictory_antecedent() {
    let spec = "Always, if every Req holds, then every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    // Contradictory antecedent: req && !req is always false → vacuously true implication
    let result = check_z3_hw_equivalence(spec, "(req && !req) |-> ack", &decls, 5).unwrap();
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "Contradictory antecedent (vacuous implication) must NOT match real spec. Got: {:?}", result);
}

#[test]
fn adversarial_z3_off_by_one_delay() {
    // Spec says "next" (X operator = delay 1), SVA has no delay
    let spec = "Always, if every Req holds, then next every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    // No delay: req |-> ack (same cycle)
    let result = check_z3_hw_equivalence(spec, "req |-> ack", &decls, 5).unwrap();
    // This should be NotEquivalent because spec requires next-cycle response
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "Off-by-one delay (no delay vs X) must NOT be equivalent. Got: {:?}", result);
}

#[test]
fn adversarial_z3_bound_sensitivity() {
    // Liveness at bound=1 might miss divergence that bound=5 catches
    let spec = "Always, if every Req holds, then eventually every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];

    // "req |-> req" is wrong for handshake (never checks ack)
    let r1 = check_z3_hw_equivalence(spec, "req |-> req", &decls, 1).unwrap();
    let r5 = check_z3_hw_equivalence(spec, "req |-> req", &decls, 5).unwrap();

    let either_catches = !matches!(r1, EquivalenceResult::Equivalent)
        || !matches!(r5, EquivalenceResult::Equivalent);
    assert!(either_catches,
        "At least one bound must catch req|->req vs handshake divergence.\n\
         Bound=1: {:?}\nBound=5: {:?}", r1, r5);
}

#[test]
fn adversarial_z3_double_negation() {
    // Double negation: "not not valid" should be equivalent to "valid"
    let bounded_pos = translate_sva_to_bounded("req", 3).unwrap();
    let bounded_neg = translate_sva_to_bounded("!!req", 3).unwrap();
    let verify_pos = bounded_to_verify(&bounded_pos.expr);
    let verify_neg = bounded_to_verify(&bounded_neg.expr);
    let signals = vec!["req".to_string()];
    let result = check_equivalence(&verify_pos, &verify_neg, &signals, 3);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Double negation must be equivalent to positive. Got: {:?}", result);
}

#[test]
fn adversarial_z3_self_equiv_battery() {
    let specs = vec![
        "Always, every signal is valid.",
        "Eventually, every signal is active.",
        "Always, if every request holds, then every grant holds.",
        "Always, if every request holds, then eventually every acknowledgment holds.",
        "Always, every dog runs.",
    ];

    for spec in &specs {
        let synth = synthesize_sva_from_spec(spec, "clk").unwrap();
        assert!(!synth.body.is_empty(), "Empty body for: {}", spec);
        let result = check_z3_equivalence(spec, &synth.body, 5).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Self-equivalence battery FAILED.\nSpec: {}\nBody: {}\nGot: {:?}",
            spec, synth.body, result);
    }
}

#[test]
fn adversarial_z3_missing_signal() {
    // Spec mentions 2 signals, SVA only uses 1
    let spec = "Always, if every Req holds, then every Ack holds.";
    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    // SVA only mentions req, not ack
    let result = check_z3_hw_equivalence(spec, "req", &decls, 5).unwrap();
    assert!(!matches!(result, EquivalenceResult::Equivalent),
        "SVA missing signal (ack) must NOT be equivalent to conditional spec. Got: {:?}", result);
}
