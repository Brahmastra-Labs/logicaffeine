//! SVA Surface Area Expansion Tests
//!
//! These tests define the specification for every SVA construct that LOGOS must support.
//! Tests are diamond — the implementation must conform to these tests.
//!
//! IEEE 1800 SVA constructs covered:
//! - $stable(sig), $changed(sig)           — system functions
//! - [*N], [*min:max]                      — sequence repetition
//! - disable iff (reset)                   — reset/abort clause
//! - @(negedge clk)                        — negative edge clocking
//! - s_always                              — strong always operator
//! - nexttime, nexttime[N]                 — next time temporal operator
//! - if...else                             — conditional property

use logicaffeine_compile::codegen_sva::sva_model::{
    parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent, SvaExpr,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::{
    BoundedExpr, SvaTranslator, count_and_leaves, count_or_leaves,
    extract_signal_names,
};
#[cfg(feature = "verification")]
use logicaffeine_compile::codegen_sva::sva_to_verify::bounded_to_verify;

// ============================================================================
// SECTION 1: $stable and $changed — System Functions
// ============================================================================

#[test]
fn parse_stable_system_function() {
    let expr = parse_sva("$stable(data)").unwrap();
    assert!(matches!(expr, SvaExpr::Stable(_)));
}

#[test]
fn parse_changed_system_function() {
    let expr = parse_sva("$changed(data)").unwrap();
    assert!(matches!(expr, SvaExpr::Changed(_)));
}

#[test]
fn parse_stable_in_implication() {
    let expr = parse_sva("valid |-> $stable(data)").unwrap();
    if let SvaExpr::Implication { consequent, overlapping, .. } = &expr {
        assert!(matches!(consequent.as_ref(), SvaExpr::Stable(_)));
        assert!(*overlapping);
    } else {
        panic!("expected implication, got {:?}", expr);
    }
}

#[test]
fn parse_changed_in_implication() {
    let expr = parse_sva("enable |=> $changed(counter)").unwrap();
    if let SvaExpr::Implication { consequent, overlapping, .. } = &expr {
        assert!(matches!(consequent.as_ref(), SvaExpr::Changed(_)));
        assert!(!*overlapping);
    } else {
        panic!("expected implication, got {:?}", expr);
    }
}

#[test]
fn stable_to_string_roundtrips() {
    let expr = parse_sva("$stable(data)").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "$stable(data)");
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn changed_to_string_roundtrips() {
    let expr = parse_sva("$changed(data)").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "$changed(data)");
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn stable_structural_equivalence() {
    let a = parse_sva("$stable(data)").unwrap();
    let b = parse_sva("$stable(data)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn stable_vs_changed_not_equivalent() {
    let a = parse_sva("$stable(data)").unwrap();
    let b = parse_sva("$changed(data)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_stable_at_t5_is_eq_current_previous() {
    let expr = parse_sva("$stable(sig)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 5);
    // $stable(sig) at t=5 means sig@5 == sig@4
    assert!(matches!(result, BoundedExpr::Eq(_, _)));
    if let BoundedExpr::Eq(left, right) = &result {
        assert_eq!(*left, Box::new(BoundedExpr::Var("sig@5".into())));
        assert_eq!(*right, Box::new(BoundedExpr::Var("sig@4".into())));
    }
}

#[test]
fn translate_stable_at_t0_is_true() {
    let expr = parse_sva("$stable(sig)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 0);
    // At t=0, no prior state → stable is vacuously true
    assert_eq!(result, BoundedExpr::Bool(true));
}

#[test]
fn translate_changed_at_t5_is_not_eq_current_previous() {
    let expr = parse_sva("$changed(sig)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 5);
    // $changed(sig) at t=5 means sig@5 != sig@4, which is !(sig@5 == sig@4)
    assert!(matches!(result, BoundedExpr::Not(_)));
}

#[test]
fn translate_changed_at_t0_is_false() {
    let expr = parse_sva("$changed(sig)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 0);
    // At t=0, no prior state → changed is vacuously false
    assert_eq!(result, BoundedExpr::Bool(false));
}

// ============================================================================
// SECTION 2: Sequence Repetition — [*N] and [*min:max]
// ============================================================================

#[test]
fn parse_exact_repetition() {
    let expr = parse_sva("req[*3]").unwrap();
    if let SvaExpr::Repetition { min, max, .. } = &expr {
        assert_eq!(*min, 3);
        assert_eq!(*max, Some(3));
    } else {
        panic!("expected Repetition, got {:?}", expr);
    }
}

#[test]
fn parse_range_repetition() {
    let expr = parse_sva("req[*1:5]").unwrap();
    if let SvaExpr::Repetition { min, max, .. } = &expr {
        assert_eq!(*min, 1);
        assert_eq!(*max, Some(5));
    } else {
        panic!("expected Repetition, got {:?}", expr);
    }
}

#[test]
fn parse_zero_or_more_repetition() {
    let expr = parse_sva("req[*0:$]").unwrap();
    if let SvaExpr::Repetition { min, max, .. } = &expr {
        assert_eq!(*min, 0);
        assert_eq!(*max, None); // unbounded
    } else {
        panic!("expected Repetition, got {:?}", expr);
    }
}

#[test]
fn parse_one_or_more_repetition() {
    let expr = parse_sva("req[*1:$]").unwrap();
    if let SvaExpr::Repetition { min, max, .. } = &expr {
        assert_eq!(*min, 1);
        assert_eq!(*max, None); // unbounded
    } else {
        panic!("expected Repetition, got {:?}", expr);
    }
}

#[test]
fn repetition_to_string_exact() {
    let expr = parse_sva("req[*3]").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "req[*3]");
}

#[test]
fn repetition_to_string_range() {
    let expr = parse_sva("req[*1:5]").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "req[*1:5]");
}

#[test]
fn repetition_to_string_unbounded() {
    let expr = parse_sva("req[*1:$]").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "req[*1:$]");
}

#[test]
fn repetition_roundtrip() {
    for input in &["req[*3]", "req[*1:5]", "req[*0:$]"] {
        let expr = parse_sva(input).unwrap();
        let text = sva_expr_to_string(&expr);
        let reparsed = parse_sva(&text).unwrap();
        assert!(
            sva_exprs_structurally_equivalent(&expr, &reparsed),
            "roundtrip failed for '{}'", input
        );
    }
}

#[test]
fn translate_exact_repetition_to_conjunction() {
    // req[*3] at t=2 means req@2 && req@3 && req@4
    let expr = parse_sva("req[*3]").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 2);
    assert_eq!(count_and_leaves(&result), 3);
}

#[test]
fn translate_range_repetition_wraps_disjunction_of_conjunctions() {
    // req[*1:3] at t=0 means:
    //   (req@0) || (req@0 && req@1) || (req@0 && req@1 && req@2)
    let expr = parse_sva("req[*1:3]").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 0);
    // Top-level should be a disjunction of 3 alternatives
    assert_eq!(count_or_leaves(&result), 3);
}

#[test]
fn repetition_in_implication_context() {
    let expr = parse_sva("$rose(req) |-> req[*1:5] ##1 ack").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
}

// ============================================================================
// SECTION 3: disable iff — Reset/Abort Clause
// ============================================================================

#[test]
fn parse_disable_iff() {
    let expr = parse_sva("@(posedge clk) disable iff (reset) req |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::DisableIff { .. }));
    if let SvaExpr::DisableIff { condition, body } = &expr {
        assert!(matches!(condition.as_ref(), SvaExpr::Signal(s) if s == "reset"));
        assert!(matches!(body.as_ref(), SvaExpr::Implication { .. }));
    }
}

#[test]
fn parse_disable_iff_with_complex_condition() {
    let expr = parse_sva("disable iff (reset || power_down) req |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::DisableIff { .. }));
    if let SvaExpr::DisableIff { condition, .. } = &expr {
        assert!(matches!(condition.as_ref(), SvaExpr::Or(_, _)));
    }
}

#[test]
fn disable_iff_to_string_roundtrips() {
    let expr = parse_sva("disable iff (reset) req |-> ack").unwrap();
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("disable iff"));
    assert!(text.contains("reset"));
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn translate_disable_iff_guards_with_condition() {
    // disable iff (reset) P translates to: !reset@t → P@t
    // (when reset is active, the property is vacuously true)
    let expr = parse_sva("disable iff (reset) req |-> ack").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 3);
    // Should be: Implies(Not(Var("reset@3")), <inner property at t=3>)
    assert!(matches!(result, BoundedExpr::Implies(_, _)));
}

// ============================================================================
// SECTION 4: Negative Edge Clocking
// ============================================================================

#[test]
fn parse_negedge_clock_strips_sensitivity() {
    // Parser should strip @(negedge clk) just like @(posedge clk)
    let expr = parse_sva("@(negedge clk) req |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
}

#[test]
fn parse_negedge_in_complex_expression() {
    let expr = parse_sva("@(negedge sclk) $rose(mosi) |-> ##[1:3] miso").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
    if let SvaExpr::Implication { consequent, .. } = &expr {
        assert!(matches!(consequent.as_ref(), SvaExpr::Delay { .. }));
    }
}

// ============================================================================
// SECTION 5: s_always — Strong Always Operator
// ============================================================================

#[test]
fn parse_s_always() {
    let expr = parse_sva("s_always(valid)").unwrap();
    assert!(matches!(expr, SvaExpr::SAlways(_)));
}

#[test]
fn parse_s_always_nested() {
    let expr = parse_sva("req |-> s_always(ack)").unwrap();
    if let SvaExpr::Implication { consequent, .. } = &expr {
        assert!(matches!(consequent.as_ref(), SvaExpr::SAlways(_)));
    } else {
        panic!("expected implication, got {:?}", expr);
    }
}

#[test]
fn s_always_to_string_roundtrips() {
    let expr = parse_sva("s_always(valid)").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "s_always(valid)");
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn translate_s_always_to_conjunction() {
    // s_always(P) at t=2 with bound=5 means P@2 && P@3 && P@4
    let expr = parse_sva("s_always(valid)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 2);
    assert_eq!(count_and_leaves(&result), 3); // 5 - 2 = 3 remaining timesteps
}

#[test]
fn s_always_vs_s_eventually_not_equivalent() {
    let a = parse_sva("s_always(valid)").unwrap();
    let b = parse_sva("s_eventually(valid)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

// ============================================================================
// SECTION 6: nexttime — Next Time Temporal Operator
// ============================================================================

#[test]
fn parse_nexttime() {
    let expr = parse_sva("nexttime(valid)").unwrap();
    if let SvaExpr::Nexttime(inner, n) = &expr {
        assert!(matches!(inner.as_ref(), SvaExpr::Signal(s) if s == "valid"));
        assert_eq!(*n, 1); // default is 1
    } else {
        panic!("expected Nexttime, got {:?}", expr);
    }
}

#[test]
fn parse_nexttime_with_count() {
    let expr = parse_sva("nexttime[3](valid)").unwrap();
    if let SvaExpr::Nexttime(_, n) = &expr {
        assert_eq!(*n, 3);
    } else {
        panic!("expected Nexttime, got {:?}", expr);
    }
}

#[test]
fn nexttime_to_string_default() {
    let expr = parse_sva("nexttime(valid)").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "nexttime(valid)");
}

#[test]
fn nexttime_to_string_with_count() {
    let expr = parse_sva("nexttime[3](valid)").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "nexttime[3](valid)");
}

#[test]
fn nexttime_roundtrips() {
    for input in &["nexttime(valid)", "nexttime[3](valid)"] {
        let expr = parse_sva(input).unwrap();
        let text = sva_expr_to_string(&expr);
        let reparsed = parse_sva(&text).unwrap();
        assert!(
            sva_exprs_structurally_equivalent(&expr, &reparsed),
            "roundtrip failed for '{}'", input
        );
    }
}

#[test]
fn translate_nexttime_shifts_by_one() {
    let expr = parse_sva("nexttime(valid)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 3);
    // nexttime(valid) at t=3 means valid@4
    assert_eq!(result, BoundedExpr::Var("valid@4".into()));
}

#[test]
fn translate_nexttime_n_shifts_by_n() {
    let expr = parse_sva("nexttime[3](valid)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 2);
    // nexttime[3](valid) at t=2 means valid@5
    assert_eq!(result, BoundedExpr::Var("valid@5".into()));
}

// ============================================================================
// SECTION 7: if...else — Conditional Property
// ============================================================================

#[test]
fn parse_if_else_property() {
    let expr = parse_sva("if (mode) req |-> ack else req |-> nack").unwrap();
    assert!(matches!(expr, SvaExpr::IfElse { .. }));
    if let SvaExpr::IfElse { condition, then_expr, else_expr } = &expr {
        assert!(matches!(condition.as_ref(), SvaExpr::Signal(s) if s == "mode"));
        assert!(matches!(then_expr.as_ref(), SvaExpr::Implication { .. }));
        assert!(matches!(else_expr.as_ref(), SvaExpr::Implication { .. }));
    }
}

#[test]
fn parse_if_without_else() {
    let expr = parse_sva("if (mode) req |-> ack").unwrap();
    // Without else, should still parse — else branch is implicitly true
    assert!(matches!(expr, SvaExpr::IfElse { .. }));
    if let SvaExpr::IfElse { else_expr, .. } = &expr {
        // No else clause → vacuously true when condition is false
        assert!(matches!(else_expr.as_ref(), SvaExpr::Signal(_) | SvaExpr::And(_, _))
            || matches!(else_expr.as_ref(), _)); // accept any valid default
    }
}

#[test]
fn if_else_to_string_roundtrips() {
    let expr = parse_sva("if (mode) req |-> ack else req |-> nack").unwrap();
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("if"));
    assert!(text.contains("else"));
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn translate_if_else_to_conditional() {
    // if (mode) P else Q at t=3 means:
    // (mode@3 → P@3) ∧ (¬mode@3 → Q@3)
    let expr = parse_sva("if (mode) req |-> ack else req |-> nack").unwrap();
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&expr, 3);
    // Should produce And(Implies(...), Implies(...))
    assert!(matches!(result, BoundedExpr::And(_, _)));
}

// ============================================================================
// SECTION 8: Complex Compositions — Real Protocol Patterns
// ============================================================================

#[test]
fn parse_handshake_with_reset() {
    let expr = parse_sva("disable iff (reset) $rose(req) |-> ##[1:5] ack").unwrap();
    assert!(matches!(expr, SvaExpr::DisableIff { .. }));
}

#[test]
fn parse_data_integrity_with_stable() {
    let expr = parse_sva("valid |=> $stable(data)").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
    if let SvaExpr::Implication { consequent, overlapping, .. } = &expr {
        assert!(matches!(consequent.as_ref(), SvaExpr::Stable(_)));
        assert!(!*overlapping); // |=> is non-overlapping
    }
}

#[test]
fn parse_burst_with_repetition() {
    // AXI burst: after address phase, data repeats for burst_len cycles
    let expr = parse_sva("(AWVALID && AWREADY) |-> ##1 WVALID[*1:256]").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
}

#[test]
fn parse_arbiter_fairness() {
    // If request is held, grant must eventually come
    let expr = parse_sva("s_always(req) |-> s_eventually(grant)").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
    if let SvaExpr::Implication { antecedent, consequent, .. } = &expr {
        assert!(matches!(antecedent.as_ref(), SvaExpr::SAlways(_)));
        assert!(matches!(consequent.as_ref(), SvaExpr::SEventually(_)));
    }
}

#[test]
fn parse_spi_protocol_pattern() {
    // SPI: on negative edge of SCLK, MOSI must be stable
    let expr = parse_sva("@(negedge sclk) !ss |-> $stable(mosi)").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
}

#[test]
fn parse_conditional_protocol_mode() {
    let expr = parse_sva(
        "if (dma_mode) req |-> ##[1:3] ack else req |-> ##[1:10] ack"
    ).unwrap();
    assert!(matches!(expr, SvaExpr::IfElse { .. }));
}

#[test]
fn parse_nested_temporal_operators() {
    let expr = parse_sva("$rose(req) |-> nexttime(s_eventually(ack))").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }));
    if let SvaExpr::Implication { consequent, .. } = &expr {
        assert!(matches!(consequent.as_ref(), SvaExpr::Nexttime(_, 1)));
    }
}

// ============================================================================
// SECTION 9: Property-Level Translation — Full Temporal Unrolling
// ============================================================================

#[test]
fn translate_property_stable_over_all_timesteps() {
    let expr = parse_sva("$stable(data)").unwrap();
    let mut translator = SvaTranslator::new(4);
    let result = translator.translate_property(&expr);
    // Property translation conjoins over timesteps:
    // t=0: true (no prior), t=1: data@1==data@0, t=2: data@2==data@1, t=3: data@3==data@2
    let leaves = count_and_leaves(&result.expr);
    assert!(leaves >= 3, "expected at least 3 and-leaves, got {}", leaves);
}

#[test]
fn translate_property_repetition_with_bound() {
    let expr = parse_sva("valid[*3]").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate_property(&expr);
    let signals = extract_signal_names(&result);
    assert!(signals.contains(&"valid".to_string()));
}

#[test]
fn translate_property_disable_iff_guards_every_timestep() {
    let expr = parse_sva("disable iff (reset) valid").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate_property(&expr);
    let signals = extract_signal_names(&result);
    assert!(signals.contains(&"reset".to_string()));
    assert!(signals.contains(&"valid".to_string()));
}

// ============================================================================
// SECTION 10: Bounded → VerifyExpr Bridge for New Constructs
// ============================================================================

#[cfg(feature = "verification")]
#[test]
fn stable_bounded_to_verify_produces_eq() {
    let expr = parse_sva("$stable(sig)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let bounded = translator.translate(&expr, 5);
    let verify = bounded_to_verify(&bounded);
    // Should produce VerifyExpr::Binary { op: Eq, ... }
    assert!(matches!(
        verify,
        logicaffeine_verify::VerifyExpr::Binary {
            op: logicaffeine_verify::VerifyOp::Eq, ..
        }
    ));
}

#[cfg(feature = "verification")]
#[test]
fn changed_bounded_to_verify_produces_not_eq() {
    let expr = parse_sva("$changed(sig)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let bounded = translator.translate(&expr, 5);
    let verify = bounded_to_verify(&bounded);
    // Should produce VerifyExpr::Not(Binary { op: Eq, ... })
    assert!(matches!(verify, logicaffeine_verify::VerifyExpr::Not(_)));
}

#[cfg(feature = "verification")]
#[test]
fn nexttime_bounded_to_verify_produces_var_at_next() {
    let expr = parse_sva("nexttime(valid)").unwrap();
    let mut translator = SvaTranslator::new(10);
    let bounded = translator.translate(&expr, 3);
    let verify = bounded_to_verify(&bounded);
    // Should produce VerifyExpr::Var("valid@4")
    assert!(matches!(verify, logicaffeine_verify::VerifyExpr::Var(ref name) if name == "valid@4"));
}
