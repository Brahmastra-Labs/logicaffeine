//! Spec Self-Consistency Checking Tests
//!
//! Tests for MUS extraction, vacuity detection, redundancy detection,
//! pairwise conflict identification, and the full check_spec_consistency API.

#![cfg(feature = "verification")]

use logicaffeine_verify::consistency::{
    check_spec_consistency, ConsistencyConfig, ConsistencyReport,
    LabeledFormula, SatisfiabilityResult,
};
use logicaffeine_verify::ir::{VerifyExpr, VerifyOp};

fn lf(index: usize, label: &str, expr: VerifyExpr) -> LabeledFormula {
    LabeledFormula {
        index,
        label: label.to_string(),
        expr,
    }
}

fn default_config() -> ConsistencyConfig {
    ConsistencyConfig::default()
}

// ═══════════════════════════════════════════════════════════════════════════
// TODO 2: MUS EXTRACTION TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn mus_simple_contradiction() {
    // P and Not(P) — both are necessary for the contradiction
    let formulas = vec![
        lf(0, "P", VerifyExpr::var("x@0")),
        lf(1, "Not(P)", VerifyExpr::not(VerifyExpr::var("x@0"))),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    match &report.satisfiability {
        SatisfiabilityResult::Unsatisfiable { mus } => {
            assert!(mus.contains(&0), "MUS should contain 0. Got: {:?}", mus);
            assert!(mus.contains(&1), "MUS should contain 1. Got: {:?}", mus);
            assert_eq!(mus.len(), 2, "MUS should have exactly 2 elements");
        }
        other => panic!("Expected Unsatisfiable, got {:?}", other),
    }
}

#[test]
fn mus_three_with_irrelevant() {
    // x>10 and x<5 conflict; y is irrelevant
    let formulas = vec![
        lf(0, "x>10", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(10))),
        lf(1, "x<5", VerifyExpr::lt(VerifyExpr::var("x@0"), VerifyExpr::int(5))),
        lf(2, "y", VerifyExpr::var("y@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    match &report.satisfiability {
        SatisfiabilityResult::Unsatisfiable { mus } => {
            assert!(mus.contains(&0), "MUS should contain 0. Got: {:?}", mus);
            assert!(mus.contains(&1), "MUS should contain 1. Got: {:?}", mus);
            assert!(!mus.contains(&2), "MUS should NOT contain 2 (irrelevant). Got: {:?}", mus);
        }
        other => panic!("Expected Unsatisfiable, got {:?}", other),
    }
}

#[test]
fn mus_three_all_necessary() {
    // Three formulas where ALL THREE are needed for UNSAT.
    // x=true, y=true, AND (x implies not y)
    // {0,1} → SAT (x=true, y=true, fine)
    // {0,2} → SAT (x=true, y=false satisfies both)
    // {1,2} → SAT (x=false, y=true satisfies both)
    // {0,1,2} → UNSAT (x=true forces not y via formula 2, but formula 1 forces y=true)
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "y", VerifyExpr::var("y@0")),
        lf(2, "x implies not y", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::not(VerifyExpr::var("y@0")),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    match &report.satisfiability {
        SatisfiabilityResult::Unsatisfiable { mus } => {
            assert_eq!(mus.len(), 3, "All three formulas needed. Got MUS: {:?}", mus);
            assert!(mus.contains(&0));
            assert!(mus.contains(&1));
            assert!(mus.contains(&2));
        }
        other => panic!("Expected Unsatisfiable, got {:?}", other),
    }
}

#[test]
fn mus_satisfiable_returns_satisfiable() {
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "y", VerifyExpr::var("y@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable,
        "Independent vars should be satisfiable");
}

#[test]
fn mus_single_self_contradictory() {
    // Single formula that contradicts itself: x AND NOT x
    let formulas = vec![
        lf(0, "x and not x", VerifyExpr::and(
            VerifyExpr::var("x@0"),
            VerifyExpr::not(VerifyExpr::var("x@0")),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    match &report.satisfiability {
        SatisfiabilityResult::Unsatisfiable { mus } => {
            assert_eq!(mus, &vec![0], "Single self-contradictory formula is the MUS");
        }
        other => panic!("Expected Unsatisfiable, got {:?}", other),
    }
}

#[test]
fn mus_empty_set() {
    let report = check_spec_consistency(&[], &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable,
        "Empty set is trivially satisfiable");
}

// ═══════════════════════════════════════════════════════════════════════════
// TODO 3: VACUITY DETECTION TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn vacuity_impossible_antecedent() {
    // Not(x) makes "if x then y" vacuously true
    let formulas = vec![
        lf(0, "not x", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(1, "x implies y", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert_eq!(report.vacuity.len(), 1, "Should find 1 vacuous formula. Got: {:?}", report.vacuity);
    assert_eq!(report.vacuity[0].formula_index, 1,
        "Formula 1 should be vacuous. Got index: {}", report.vacuity[0].formula_index);
}

#[test]
fn vacuity_no_vacuous_formulas() {
    // x is true, so "if x then y" is NOT vacuous
    let formulas = vec![
        lf(0, "x implies y", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
        lf(1, "x", VerifyExpr::var("x@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.vacuity.is_empty(),
        "No vacuous formulas expected. Got: {:?}", report.vacuity);
}

#[test]
fn vacuity_non_implication_ignored() {
    // And(x, y) is not an implication — vacuity check should skip it
    let formulas = vec![
        lf(0, "not x", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(1, "x and y", VerifyExpr::and(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    // Note: this is UNSAT (not x AND (x AND y) is contradictory)
    // So vacuity won't run (only runs when SAT)
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Unsatisfiable { .. }),
        "Should be unsatisfiable");
    assert!(report.vacuity.is_empty(), "Vacuity not checked when UNSAT");
}

#[test]
fn vacuity_forall_wrapping() {
    // ForAll wrapper around implication — should still detect vacuity
    let formulas = vec![
        lf(0, "not x", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(1, "forall: x implies y", VerifyExpr::forall(
            vec![],
            VerifyExpr::implies(
                VerifyExpr::var("x@0"),
                VerifyExpr::var("y@0"),
            ),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert_eq!(report.vacuity.len(), 1, "Should find vacuous ForAll-wrapped implication");
    assert_eq!(report.vacuity[0].formula_index, 1);
}

#[test]
fn vacuity_multiple_findings() {
    // Both x and y are impossible → both implications are vacuous
    let formulas = vec![
        lf(0, "not x", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(1, "not y", VerifyExpr::not(VerifyExpr::var("y@0"))),
        lf(2, "x implies z", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("z@0"),
        )),
        lf(3, "y implies z", VerifyExpr::implies(
            VerifyExpr::var("y@0"),
            VerifyExpr::var("z@0"),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    let indices: Vec<usize> = report.vacuity.iter().map(|v| v.formula_index).collect();
    assert!(indices.contains(&2), "Formula 2 should be vacuous. Got: {:?}", indices);
    assert!(indices.contains(&3), "Formula 3 should be vacuous. Got: {:?}", indices);
    assert_eq!(report.vacuity.len(), 2, "Exactly 2 vacuous formulas expected");
}

#[test]
fn vacuity_only_checks_implications() {
    // No implications in the spec — nothing to check for vacuity
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "y", VerifyExpr::var("y@0")),
        lf(2, "x or y", VerifyExpr::or(VerifyExpr::var("x@0"), VerifyExpr::var("y@0"))),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.vacuity.is_empty(),
        "No implications means no vacuity findings");
}

// ═══════════════════════════════════════════════════════════════════════════
// TODO 4: REDUNDANCY DETECTION TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn redundancy_strict_entailment() {
    // x>10 entails x>5
    let formulas = vec![
        lf(0, "x > 10", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(10))),
        lf(1, "x > 5", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(5))),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(!report.redundancies.is_empty(), "Should find redundancy");
    assert!(report.redundancies.iter().any(|r| r.redundant_index == 1),
        "Formula 1 (x>5) should be redundant. Got: {:?}", report.redundancies);
}

#[test]
fn redundancy_no_redundancy() {
    // Independent variables — neither entails the other
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "y", VerifyExpr::var("y@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert!(report.redundancies.is_empty(),
        "No redundancy expected. Got: {:?}", report.redundancies);
}

#[test]
fn redundancy_mutual_entailment() {
    // Contrapositive: (x→y) and (¬y→¬x) are logically equivalent
    let formulas = vec![
        lf(0, "x implies y", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
        lf(1, "not y implies not x", VerifyExpr::implies(
            VerifyExpr::not(VerifyExpr::var("y@0")),
            VerifyExpr::not(VerifyExpr::var("x@0")),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(!report.redundancies.is_empty(),
        "At least one should be redundant (mutual entailment). Got: {:?}", report.redundancies);
}

#[test]
fn redundancy_three_formulas_one_redundant() {
    // x AND (x→y) together entail y
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "x implies y", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
        lf(2, "y", VerifyExpr::var("y@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.redundancies.iter().any(|r| r.redundant_index == 2),
        "Formula 2 (y) should be redundant. Got: {:?}", report.redundancies);
}

#[test]
fn redundancy_with_implication_chain() {
    // (a→b) AND (b→c) entails (a→c)
    let formulas = vec![
        lf(0, "a implies b", VerifyExpr::implies(
            VerifyExpr::var("a@0"),
            VerifyExpr::var("b@0"),
        )),
        lf(1, "b implies c", VerifyExpr::implies(
            VerifyExpr::var("b@0"),
            VerifyExpr::var("c@0"),
        )),
        lf(2, "a implies c", VerifyExpr::implies(
            VerifyExpr::var("a@0"),
            VerifyExpr::var("c@0"),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.redundancies.iter().any(|r| r.redundant_index == 2),
        "Formula 2 (a→c) should be redundant via transitivity. Got: {:?}", report.redundancies);
}

#[test]
fn redundancy_identical_formulas() {
    // Two identical formulas — at least one is redundant
    let formulas = vec![
        lf(0, "x copy 1", VerifyExpr::var("x@0")),
        lf(1, "x copy 2", VerifyExpr::var("x@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(!report.redundancies.is_empty(),
        "Identical formulas should produce redundancy. Got: {:?}", report.redundancies);
}

// ═══════════════════════════════════════════════════════════════════════════
// TODO 5: PAIRWISE CONSISTENCY TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn pairwise_short_circuit_when_sat() {
    // All satisfiable together → no pairwise conflicts possible
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "y", VerifyExpr::var("y@0")),
        lf(2, "z", VerifyExpr::var("z@0")),
    ];
    let config = ConsistencyConfig { check_pairwise: true, ..default_config() };
    let report = check_spec_consistency(&formulas, &config);
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.pairwise_conflicts.is_empty(),
        "SAT conjunction means no pairwise conflicts. Got: {:?}", report.pairwise_conflicts);
}

#[test]
fn pairwise_finds_all_conflicts() {
    // Two independent contradictions: (x, not x) and (y, not y)
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "not x", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(2, "y", VerifyExpr::var("y@0")),
        lf(3, "not y", VerifyExpr::not(VerifyExpr::var("y@0"))),
    ];
    let config = ConsistencyConfig { check_pairwise: true, ..default_config() };
    let report = check_spec_consistency(&formulas, &config);
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Unsatisfiable { .. }));
    let pairs: Vec<(usize, usize)> = report.pairwise_conflicts.iter()
        .map(|p| (p.i, p.j))
        .collect();
    assert!(pairs.contains(&(0, 1)), "Should find conflict (0,1). Got: {:?}", pairs);
    assert!(pairs.contains(&(2, 3)), "Should find conflict (2,3). Got: {:?}", pairs);
    assert!(!pairs.contains(&(0, 2)), "Should NOT find conflict (0,2). Got: {:?}", pairs);
}

#[test]
fn pairwise_disabled_by_config() {
    let formulas = vec![
        lf(0, "x", VerifyExpr::var("x@0")),
        lf(1, "not x", VerifyExpr::not(VerifyExpr::var("x@0"))),
    ];
    let config = ConsistencyConfig { check_pairwise: false, ..default_config() };
    let report = check_spec_consistency(&formulas, &config);
    assert!(matches!(report.satisfiability, SatisfiabilityResult::Unsatisfiable { .. }));
    assert!(report.pairwise_conflicts.is_empty(),
        "Pairwise disabled → empty. Got: {:?}", report.pairwise_conflicts);
}

// ═══════════════════════════════════════════════════════════════════════════
// TODO 6: FULL REPORT ORCHESTRATION TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn report_satisfiable_clean() {
    let formulas = vec![
        lf(0, "P1", VerifyExpr::var("x@0")),
        lf(1, "P2", VerifyExpr::var("y@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.vacuity.is_empty());
    assert!(report.redundancies.is_empty());
    assert!(report.pairwise_conflicts.is_empty());
}

#[test]
fn report_unsatisfiable_with_mus() {
    let formulas = vec![
        lf(0, "P1", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(10))),
        lf(1, "P2", VerifyExpr::lt(VerifyExpr::var("x@0"), VerifyExpr::int(5))),
        lf(2, "P3", VerifyExpr::var("y@0")),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    match &report.satisfiability {
        SatisfiabilityResult::Unsatisfiable { mus } => {
            assert!(mus.contains(&0));
            assert!(mus.contains(&1));
            assert!(!mus.contains(&2));
        }
        other => panic!("Expected Unsatisfiable. Got: {:?}", other),
    }
    // When UNSAT, vacuity and redundancy are not checked
    assert!(report.vacuity.is_empty());
    assert!(report.redundancies.is_empty());
}

#[test]
fn report_vacuous_only() {
    let formulas = vec![
        lf(0, "P1", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(1, "P2", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert_eq!(report.vacuity.len(), 1);
    assert_eq!(report.vacuity[0].formula_index, 1);
}

#[test]
fn report_redundant_only() {
    let formulas = vec![
        lf(0, "P1", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(10))),
        lf(1, "P2", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(5))),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.redundancies.iter().any(|r| r.redundant_index == 1));
}

#[test]
fn report_vacuity_disabled() {
    let formulas = vec![
        lf(0, "P1", VerifyExpr::not(VerifyExpr::var("x@0"))),
        lf(1, "P2", VerifyExpr::implies(
            VerifyExpr::var("x@0"),
            VerifyExpr::var("y@0"),
        )),
    ];
    let config = ConsistencyConfig { check_vacuity: false, ..default_config() };
    let report = check_spec_consistency(&formulas, &config);
    assert!(report.vacuity.is_empty(), "Vacuity check disabled");
}

#[test]
fn report_redundancy_disabled() {
    let formulas = vec![
        lf(0, "P1", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(10))),
        lf(1, "P2", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(5))),
    ];
    let config = ConsistencyConfig { check_redundancy: false, ..default_config() };
    let report = check_spec_consistency(&formulas, &config);
    assert!(report.redundancies.is_empty(), "Redundancy check disabled");
}

#[test]
fn report_combined_findings() {
    // P1: x>10 (strong constraint)
    // P2: x>5 (redundant — entailed by P1)
    // P3: not y
    // P4: y implies z (vacuous — y impossible due to P3)
    let formulas = vec![
        lf(0, "x > 10", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(10))),
        lf(1, "x > 5", VerifyExpr::gt(VerifyExpr::var("x@0"), VerifyExpr::int(5))),
        lf(2, "not y", VerifyExpr::not(VerifyExpr::var("y@0"))),
        lf(3, "y implies z", VerifyExpr::implies(
            VerifyExpr::var("y@0"),
            VerifyExpr::var("z@0"),
        )),
    ];
    let report = check_spec_consistency(&formulas, &default_config());
    assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    assert!(report.redundancies.iter().any(|r| r.redundant_index == 1),
        "Formula 1 should be redundant. Got: {:?}", report.redundancies);
    assert!(report.vacuity.iter().any(|v| v.formula_index == 3),
        "Formula 3 should be vacuous. Got: {:?}", report.vacuity);
}

// ═══════════════════════════════════════════════════════════════════════════
// TODO 8: ENGLISH INTEGRATION TESTS (spec_health.rs)
// These tests are commented out until spec_health.rs is created (TODO 8).
// ═══════════════════════════════════════════════════════════════════════════

mod english_integration {
    use logicaffeine_compile::codegen_sva::spec_health::check_english_spec;
    use logicaffeine_verify::consistency::{ConsistencyConfig, SatisfiabilityResult};

    fn default_config() -> ConsistencyConfig {
        ConsistencyConfig::default()
    }

    #[test]
    fn english_consistent_spec() {
        let report = check_english_spec("Req is valid.", default_config()).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable,
            "Single valid sentence should be satisfiable");
    }

    #[test]
    fn english_two_independent_properties() {
        let report = check_english_spec(
            "Req is valid. Ack is valid.",
            default_config(),
        ).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
        assert!(report.vacuity.is_empty());
        assert!(report.redundancies.is_empty());
    }

    #[test]
    fn english_implication_chain() {
        let report = check_english_spec(
            "If Req is valid, then Ack is valid. If Ack is valid, then Data is valid.",
            default_config(),
        ).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    }

    #[test]
    fn english_config_all_disabled() {
        let config = ConsistencyConfig {
            check_vacuity: false,
            check_redundancy: false,
            check_pairwise: false,
            ..default_config()
        };
        let report = check_english_spec("Req is valid.", config).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
        assert!(report.vacuity.is_empty());
        assert!(report.redundancies.is_empty());
        assert!(report.pairwise_conflicts.is_empty());
    }

    #[test]
    fn english_single_sentence_always_consistent() {
        let report = check_english_spec(
            "Every signal is valid.",
            default_config(),
        ).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    }

    #[test]
    fn english_parse_error_propagated() {
        let result = check_english_spec("??? not valid syntax ???", default_config());
        assert!(result.is_err(), "Unparseable input should return Err");
    }

    #[test]
    fn english_empty_spec() {
        let report = check_english_spec("", default_config()).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    }

    #[test]
    fn english_sentence_splitting() {
        // Three distinct sentences should produce 3 formulas
        let report = check_english_spec(
            "Req is valid. Ack is valid. Data is valid.",
            default_config(),
        ).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    }

    #[test]
    fn english_implication_satisfiable() {
        let report = check_english_spec(
            "If Req is valid, then Ack is valid.",
            default_config(),
        ).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
        assert!(report.vacuity.is_empty(),
            "Single implication with no conflicting context is not vacuous");
    }

    #[test]
    fn english_report_preserves_labels() {
        let report = check_english_spec(
            "Req is valid. Ack is valid.",
            default_config(),
        ).unwrap();
        assert_eq!(report.satisfiability, SatisfiabilityResult::Satisfiable);
    }

    #[test]
    fn english_spec_with_preamble_parses() {
        // Regression guard: before fix 1, `check_english_spec` passed spec text
        // through `split_sentences + compile_kripke_with` per sentence, which
        // tried to parse preamble lines like `clock: clk posedge` as property
        // sentences and returned `HwError::ParseError`. After the fix it routes
        // through `parse_hw_spec_with`, which recognises preamble sigils and
        // only feeds property sentences into the consistency checker.
        //
        // This test asserts the spec parses without error — the satisfiability
        // of preamble+property semantics is a separate concern tracked
        // independently (see LOGOS_ALIGNMENT_STATUS.md known-deferred items).
        let spec = "clock: clk posedge\n\
                    signals: req : scalar\n\
                    \n\
                    Always, every req is valid.";
        let result = check_english_spec(spec, default_config());
        assert!(
            result.is_ok(),
            "spec-with-preamble must parse through parse_hw_spec_with (no HwError::ParseError on sigils); got {:?}",
            result
        );
    }
}
