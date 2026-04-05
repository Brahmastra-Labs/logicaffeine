//! Sprint 1B: IEEE 1800 Extended SVA Constructs
//!
//! Tests for 10 new SvaExpr variants that extend the SVA parser to cover
//! more of the IEEE 1800-2017 SystemVerilog Assertions standard:
//!
//! - NotEq: `a != b`
//! - LessThan: `a < b`
//! - GreaterThan: `a > b`
//! - LessEqual: `a <= b`
//! - GreaterEqual: `a >= b`
//! - Ternary: `cond ? a : b`
//! - Throughout: `sig throughout seq`
//! - Within: `seq1 within seq2`
//! - FirstMatch: `first_match(seq)`
//! - Intersect: `seq1 intersect seq2`

use logicaffeine_compile::codegen_sva::sva_model::{
    parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent, SvaExpr,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::sva_to_verify::SvaTranslator;

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: NOT-EQUAL — `a != b`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_not_equal() {
    let expr = parse_sva("req != ack").unwrap();
    assert!(matches!(expr, SvaExpr::NotEq(_, _)), "Should parse != operator. Got: {:?}", expr);
}

#[test]
fn roundtrip_not_equal() {
    let expr = parse_sva("req != ack").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: COMPARISON OPERATORS — `<`, `>`, `<=`, `>=`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_less_than() {
    let expr = parse_sva("count < 10").unwrap();
    assert!(matches!(expr, SvaExpr::LessThan(_, _)), "Got: {:?}", expr);
}

#[test]
fn parse_greater_than() {
    let expr = parse_sva("count > 0").unwrap();
    assert!(matches!(expr, SvaExpr::GreaterThan(_, _)), "Got: {:?}", expr);
}

#[test]
fn parse_less_equal() {
    let expr = parse_sva("count <= max").unwrap();
    assert!(matches!(expr, SvaExpr::LessEqual(_, _)), "Got: {:?}", expr);
}

#[test]
fn parse_greater_equal() {
    let expr = parse_sva("count >= min").unwrap();
    assert!(matches!(expr, SvaExpr::GreaterEqual(_, _)), "Got: {:?}", expr);
}

#[test]
fn roundtrip_less_than() {
    let expr = parse_sva("count < 10").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

#[test]
fn roundtrip_greater_than() {
    let expr = parse_sva("count > 0").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

#[test]
fn roundtrip_less_equal() {
    let expr = parse_sva("count <= max").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

#[test]
fn roundtrip_greater_equal() {
    let expr = parse_sva("count >= min").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: TERNARY — `cond ? a : b`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_ternary() {
    let expr = parse_sva("mode ? req : idle").unwrap();
    assert!(matches!(expr, SvaExpr::Ternary { .. }), "Got: {:?}", expr);
}

#[test]
fn roundtrip_ternary() {
    let expr = parse_sva("mode ? req : idle").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: THROUGHOUT — `sig throughout seq`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_throughout() {
    let expr = parse_sva("valid throughout (req ##[1:5] ack)").unwrap();
    assert!(matches!(expr, SvaExpr::Throughout { .. }), "Got: {:?}", expr);
}

#[test]
fn roundtrip_throughout() {
    let expr = parse_sva("valid throughout (req ##[1:5] ack)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5: WITHIN — `seq1 within seq2`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_within() {
    let expr = parse_sva("(req ##1 ack) within (valid[*3:10])").unwrap();
    assert!(matches!(expr, SvaExpr::Within { .. }), "Got: {:?}", expr);
}

#[test]
fn roundtrip_within() {
    let expr = parse_sva("(req ##1 ack) within (valid[*3:10])").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 6: FIRST_MATCH — `first_match(seq)`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_first_match() {
    let expr = parse_sva("first_match(req ##[1:5] ack)").unwrap();
    assert!(matches!(expr, SvaExpr::FirstMatch(_)), "Got: {:?}", expr);
}

#[test]
fn roundtrip_first_match() {
    let expr = parse_sva("first_match(req ##[1:5] ack)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 7: INTERSECT — `seq1 intersect seq2`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_intersect() {
    let expr = parse_sva("(req ##[1:3] ack) intersect (valid[*2:4])").unwrap();
    assert!(matches!(expr, SvaExpr::Intersect { .. }), "Got: {:?}", expr);
}

#[test]
fn roundtrip_intersect() {
    let expr = parse_sva("(req ##[1:3] ack) intersect (valid[*2:4])").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert_eq!(sva_expr_to_string(&reparsed), sva_expr_to_string(&expr));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 8: TRANSLATION TO BOUNDED EXPR
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn translate_not_equal_to_bounded() {
    let sva = parse_sva("req != ack").unwrap();
    let mut translator = logicaffeine_compile::codegen_sva::sva_to_verify::SvaTranslator::new(3);
    let result = translator.translate(&sva, 0);
    // NotEq should translate to Not(Eq(...))
    assert!(matches!(result, BoundedExpr::Not(_)), "NotEq should translate to Not. Got: {:?}", result);
}

#[test]
fn translate_less_than_to_bounded() {
    let sva = parse_sva("count < 10").unwrap();
    let mut translator = logicaffeine_compile::codegen_sva::sva_to_verify::SvaTranslator::new(3);
    let result = translator.translate(&sva, 0);
    // LessThan should produce a bounded variable comparison
    assert!(!matches!(result, BoundedExpr::Bool(true)), "LessThan should not be vacuously true");
}

#[test]
fn translate_ternary_to_bounded() {
    let sva = parse_sva("mode ? req : idle").unwrap();
    let mut translator = logicaffeine_compile::codegen_sva::sva_to_verify::SvaTranslator::new(3);
    let result = translator.translate(&sva, 0);
    // Ternary should translate to (cond ∧ then) ∨ (¬cond ∧ else)
    assert!(matches!(result, BoundedExpr::Or(_, _) | BoundedExpr::And(_, _)),
        "Ternary should translate to conditional. Got: {:?}", result);
}

#[test]
fn translate_throughout_to_bounded() {
    let sva = parse_sva("valid throughout (req ##[1:5] ack)").unwrap();
    let mut translator = logicaffeine_compile::codegen_sva::sva_to_verify::SvaTranslator::new(5);
    let result = translator.translate(&sva, 0);
    // Throughout with a range-delay inner sequence: produces an Or of And chains
    // (one per possible match endpoint). Each And chain conjoins the signal at
    // every timestep AND the sequence condition at that match length.
    let debug = format!("{:?}", result);
    assert!(debug.contains("valid@0") && debug.contains("req@0") && debug.contains("ack@"),
        "Throughout should reference valid, req, and ack at timesteps. Got: {:?}", result);
}

#[test]
fn translate_first_match_to_bounded() {
    let sva = parse_sva("first_match(req ##[1:5] ack)").unwrap();
    let mut translator = logicaffeine_compile::codegen_sva::sva_to_verify::SvaTranslator::new(5);
    let result = translator.translate(&sva, 0);
    // FirstMatch is the inner sequence (first matching length)
    assert!(!matches!(result, BoundedExpr::Bool(true)), "FirstMatch should not be vacuously true");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 9: Z3 ALGEBRAIC IDENTITIES (feature-gated)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_ieee1800 {
    use logicaffeine_verify::equivalence::{check_equivalence, EquivalenceResult};
    use logicaffeine_compile::codegen_sva::sva_to_verify::bounded_to_verify;
    use logicaffeine_compile::codegen_sva::hw_pipeline::translate_sva_to_bounded;

    #[test]
    fn z3_not_equal_equiv_not_eq() {
        // a != b ≡ !(a == b)
        let lhs = translate_sva_to_bounded("req != ack", 3).unwrap();
        let rhs = translate_sva_to_bounded("!(req == ack)", 3).unwrap();
        let v_lhs = bounded_to_verify(&lhs.expr);
        let v_rhs = bounded_to_verify(&rhs.expr);
        let result = check_equivalence(&v_lhs, &v_rhs, &["req".into(), "ack".into()], 3);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "a!=b must equal !(a==b). Got: {:?}", result);
    }

    #[test]
    fn z3_ternary_equiv_if_else() {
        // (mode ? req : idle) ≡ (if(mode) req else idle)
        let lhs = translate_sva_to_bounded("mode ? req : idle", 3).unwrap();
        let rhs = translate_sva_to_bounded("if (mode) req else idle", 3).unwrap();
        let v_lhs = bounded_to_verify(&lhs.expr);
        let v_rhs = bounded_to_verify(&rhs.expr);
        let result = check_equivalence(&v_lhs, &v_rhs,
            &["mode".into(), "req".into(), "idle".into()], 3);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Ternary must equal if/else. Got: {:?}", result);
    }

    #[test]
    fn z3_different_comparisons_are_not_equivalent() {
        use logicaffeine_verify::ir::{VerifyExpr, VerifyOp};
        let lt = VerifyExpr::binary(VerifyOp::Lt,
            VerifyExpr::Var("x".into()), VerifyExpr::Int(5));
        let gt = VerifyExpr::binary(VerifyOp::Gt,
            VerifyExpr::Var("x".into()), VerifyExpr::Int(100));
        let result = check_equivalence(&lt, &gt, &["x".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "x<5 and x>100 MUST be NotEquivalent. If Equivalent, comparisons are broken.");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 10: SPRINT A — Comparison operators MUST produce real comparisons
// ═══════════════════════════════════════════════════════════════════════════

fn collect_bounded_vars(expr: &BoundedExpr) -> Vec<String> {
    let mut vars = Vec::new();
    collect_bounded_vars_rec(expr, &mut vars);
    vars
}

fn collect_bounded_vars_rec(expr: &BoundedExpr, vars: &mut Vec<String>) {
    match expr {
        BoundedExpr::Var(name) => vars.push(name.clone()),
        BoundedExpr::And(l, r) | BoundedExpr::Or(l, r)
        | BoundedExpr::Implies(l, r) | BoundedExpr::Eq(l, r) => {
            collect_bounded_vars_rec(l, vars);
            collect_bounded_vars_rec(r, vars);
        }
        BoundedExpr::Not(inner) => collect_bounded_vars_rec(inner, vars),
        BoundedExpr::Bool(_) | BoundedExpr::Int(_) => {}
        _ => {
            // Sprint A adds Lt/Gt/Lte/Gte — handle via debug traversal
            let dbg = format!("{:?}", expr);
            // Extract Var names from debug output as fallback
            for part in dbg.split("Var(\"") {
                if let Some(end) = part.find("\")") {
                    vars.push(part[..end].to_string());
                }
            }
        }
    }
}

fn has_no_synthetic_vars(expr: &BoundedExpr) -> bool {
    let vars = collect_bounded_vars(expr);
    !vars.iter().any(|v| v.starts_with("__lt_") || v.starts_with("__gt_")
        || v.starts_with("__le_") || v.starts_with("__ge_"))
}

#[test]
fn sva_less_than_references_actual_operands() {
    let expr = parse_sva("count < 10").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    assert!(vars.iter().any(|v| v.contains("count")),
        "LessThan must reference signal 'count', got: {:?}", vars);
    assert!(has_no_synthetic_vars(&result),
        "LessThan must NOT create synthetic __lt_ variables");
}

#[test]
fn sva_greater_than_references_actual_operands() {
    let expr = parse_sva("count > 0").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    assert!(vars.iter().any(|v| v.contains("count")),
        "GreaterThan must reference signal 'count', got: {:?}", vars);
    assert!(has_no_synthetic_vars(&result),
        "GreaterThan must NOT create synthetic __gt_ variables");
}

#[test]
fn sva_less_equal_references_both_operands() {
    let expr = parse_sva("count <= max").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    assert!(vars.iter().any(|v| v.contains("count")),
        "LessEqual must reference 'count', got: {:?}", vars);
    assert!(vars.iter().any(|v| v.contains("max")),
        "LessEqual must reference 'max', got: {:?}", vars);
    assert!(has_no_synthetic_vars(&result),
        "LessEqual must NOT create synthetic variables");
}

#[test]
fn sva_greater_equal_references_both_operands() {
    let expr = parse_sva("count >= min").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    assert!(vars.iter().any(|v| v.contains("count")),
        "GreaterEqual must reference 'count', got: {:?}", vars);
    assert!(vars.iter().any(|v| v.contains("min")),
        "GreaterEqual must reference 'min', got: {:?}", vars);
    assert!(has_no_synthetic_vars(&result),
        "GreaterEqual must NOT create synthetic variables");
}

#[test]
fn sva_comparison_at_nonzero_timestep_uses_correct_suffix() {
    let expr = parse_sva("count < 10").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 3);
    let vars = collect_bounded_vars(&result);
    assert!(vars.iter().any(|v| v.contains("count@3")),
        "Comparison at t=3 must use count@3, got: {:?}", vars);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 11: SPRINT E — Multi-timestep operators MUST span multiple timesteps
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn throughout_produces_signal_at_multiple_timesteps() {
    let expr = parse_sva("valid throughout (req ##[1:3] ack)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    let valid_timesteps: Vec<u32> = vars.iter()
        .filter_map(|v| {
            if v.starts_with("valid@") {
                v.strip_prefix("valid@").and_then(|s| s.parse().ok())
            } else { None }
        })
        .collect();
    assert!(valid_timesteps.len() >= 2,
        "throughout MUST produce valid@ at MULTIPLE timesteps (signal holds for sequence duration).\n\
         Got valid@ at timesteps: {:?}\nAll vars: {:?}", valid_timesteps, vars);
}

#[test]
fn within_references_inner_and_outer_at_multiple_timesteps() {
    let expr = parse_sva("(req ##1 ack) within (valid[*3:5])").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    let req_count = vars.iter().filter(|v| v.starts_with("req@")).count();
    let valid_count = vars.iter().filter(|v| v.starts_with("valid@")).count();
    assert!(req_count >= 1, "within must reference inner sequence signals, got req@: {}", req_count);
    assert!(valid_count >= 2,
        "within's outer [*3:5] MUST produce valid@ at multiple timesteps, got: {}", valid_count);
}

#[test]
fn intersect_references_both_sequences_signals() {
    let expr = parse_sva("(req ##[1:3] ack) intersect (valid[*2:4])").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    assert!(vars.iter().any(|v| v.starts_with("req@")),
        "intersect must translate left sequence: {:?}", vars);
    assert!(vars.iter().any(|v| v.starts_with("valid@")),
        "intersect must translate right sequence: {:?}", vars);
}

#[test]
fn throughout_at_nonzero_start_still_spans() {
    let expr = parse_sva("valid throughout (req ##1 ack)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 2);
    let vars = collect_bounded_vars(&result);
    let valid_timesteps: Vec<u32> = vars.iter()
        .filter_map(|v| v.strip_prefix("valid@").and_then(|s| s.parse().ok()))
        .collect();
    assert!(valid_timesteps.len() >= 2,
        "throughout at t=2 must still span multiple timesteps, got: {:?}", valid_timesteps);
}

// ════════════════════════════���═══════════════════════════════��══════════════
// SECTION 12: $onehot0(sig) — at most one bit set
// ═══════════════════════���════════════════════════════════���══════════════════

#[test]
fn parse_onehot0_system_function() {
    let expr = parse_sva("$onehot0(grant)").unwrap();
    assert!(matches!(expr, SvaExpr::OneHot0(_)), "Got: {:?}", expr);
}

#[test]
fn onehot0_to_string() {
    let expr = parse_sva("$onehot0(grant)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$onehot0(grant)");
}

#[test]
fn onehot0_roundtrip() {
    let expr = parse_sva("$onehot0(grant)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn onehot0_structural_equiv_same() {
    let a = parse_sva("$onehot0(grant)").unwrap();
    let b = parse_sva("$onehot0(grant)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn onehot0_structural_equiv_different_signal() {
    let a = parse_sva("$onehot0(grant)").unwrap();
    let b = parse_sva("$onehot0(select)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_onehot0_produces_apply() {
    let expr = parse_sva("$onehot0(sig)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 3);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "onehot0");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], BoundedExpr::Var("sig@3".into()));
        }
        _ => panic!("$onehot0 should translate to Apply. Got: {:?}", result),
    }
}

#[test]
fn onehot0_does_not_match_onehot() {
    let a = parse_sva("$onehot0(grant)").unwrap();
    let b = parse_sva("$onehot(grant)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b),
        "$onehot0 and $onehot are DIFFERENT constructs");
}

#[test]
fn onehot0_in_implication() {
    let expr = parse_sva("$onehot0(grant) |-> $stable(data)").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }),
        "Should parse as implication. Got: {:?}", expr);
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("$onehot0(grant)"), "Emitter must preserve $onehot0. Got: {}", text);
}

#[test]
fn onehot0_in_conjunction() {
    let expr = parse_sva("$onehot0(grant_a) && $onehot0(grant_b)").unwrap();
    assert!(matches!(expr, SvaExpr::And(_, _)), "Got: {:?}", expr);
}

#[test]
fn onehot0_negated() {
    let expr = parse_sva("!($onehot0(sel))").unwrap();
    assert!(matches!(expr, SvaExpr::Not(_)), "Got: {:?}", expr);
    if let SvaExpr::Not(inner) = &expr {
        assert!(matches!(inner.as_ref(), SvaExpr::OneHot0(_)),
            "Inner should be OneHot0. Got: {:?}", inner);
    }
}

#[test]
fn translate_onehot0_at_t0() {
    let expr = parse_sva("$onehot0(sig)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "onehot0");
            assert_eq!(args[0], BoundedExpr::Var("sig@0".into()));
        }
        _ => panic!("Got: {:?}", result),
    }
}

// ══════════════════════════════���═════════════════════════════════��══════════
// SECTION 13: $onehot(sig) — exactly one bit set
// ════════════════════════════════════��══════════════════════════════════════

#[test]
fn parse_onehot_system_function() {
    let expr = parse_sva("$onehot(state)").unwrap();
    assert!(matches!(expr, SvaExpr::OneHot(_)), "Got: {:?}", expr);
}

#[test]
fn onehot_to_string() {
    let expr = parse_sva("$onehot(state)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$onehot(state)");
}

#[test]
fn onehot_roundtrip() {
    let expr = parse_sva("$onehot(state)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn onehot_structural_equiv_same() {
    let a = parse_sva("$onehot(state)").unwrap();
    let b = parse_sva("$onehot(state)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn onehot_structural_equiv_different_signal() {
    let a = parse_sva("$onehot(state)").unwrap();
    let b = parse_sva("$onehot(mode)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_onehot_produces_apply() {
    let expr = parse_sva("$onehot(state)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 2);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "onehot");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], BoundedExpr::Var("state@2".into()));
        }
        _ => panic!("$onehot should translate to Apply. Got: {:?}", result),
    }
}

#[test]
fn onehot_in_implication_body() {
    let expr = parse_sva("$onehot(state) |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }), "Got: {:?}", expr);
}

#[test]
fn onehot_negated_body() {
    let expr = parse_sva("!($onehot(state))").unwrap();
    assert!(matches!(expr, SvaExpr::Not(_)), "Got: {:?}", expr);
    if let SvaExpr::Not(inner) = &expr {
        assert!(matches!(inner.as_ref(), SvaExpr::OneHot(_)), "Got: {:?}", inner);
    }
}

#[test]
fn onehot0_before_onehot_no_confusion() {
    let oh0 = parse_sva("$onehot0(x)").unwrap();
    let oh = parse_sva("$onehot(x)").unwrap();
    assert!(matches!(oh0, SvaExpr::OneHot0(_)), "$onehot0 parsed as: {:?}", oh0);
    assert!(matches!(oh, SvaExpr::OneHot(_)), "$onehot parsed as: {:?}", oh);
}

#[test]
fn translate_onehot_at_t0() {
    let expr = parse_sva("$onehot(fsm)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "onehot");
            assert_eq!(args[0], BoundedExpr::Var("fsm@0".into()));
        }
        _ => panic!("Got: {:?}", result),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 14: $countones(sig) — population count
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_countones_system_function() {
    let expr = parse_sva("$countones(mask)").unwrap();
    assert!(matches!(expr, SvaExpr::CountOnes(_)), "Got: {:?}", expr);
}

#[test]
fn countones_to_string() {
    let expr = parse_sva("$countones(mask)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$countones(mask)");
}

#[test]
fn countones_roundtrip() {
    let expr = parse_sva("$countones(mask)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn countones_structural_equiv() {
    let a = parse_sva("$countones(mask)").unwrap();
    let b = parse_sva("$countones(mask)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_countones_produces_apply() {
    let expr = parse_sva("$countones(mask)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "countones");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], BoundedExpr::Var("mask@0".into()));
        }
        _ => panic!("$countones should translate to Apply. Got: {:?}", result),
    }
}

#[test]
fn countones_in_comparison() {
    let expr = parse_sva("$countones(mask) >= 2").unwrap();
    match &expr {
        SvaExpr::GreaterEqual(left, right) => {
            assert!(matches!(left.as_ref(), SvaExpr::CountOnes(_)),
                "LHS should be CountOnes. Got: {:?}", left);
            assert!(matches!(right.as_ref(), SvaExpr::Const(2, 32)),
                "RHS should be Const(2). Got: {:?}", right);
        }
        _ => panic!("Expected GreaterEqual(CountOnes, Const). Got: {:?}", expr),
    }
}

#[test]
fn countones_comparison_roundtrip() {
    let expr = parse_sva("$countones(mask) >= 2").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn countones_equality_comparison() {
    let expr = parse_sva("$countones(lanes) == 1").unwrap();
    assert!(matches!(expr, SvaExpr::Eq(_, _)), "Got: {:?}", expr);
}

#[test]
fn countones_less_than_comparison() {
    let expr = parse_sva("$countones(mask) < 4").unwrap();
    assert!(matches!(expr, SvaExpr::LessThan(_, _)), "Got: {:?}", expr);
}

#[test]
fn countones_in_conjunction_with_comparison() {
    let expr = parse_sva("$countones(lanes) >= 2 && $countones(lanes) <= 4").unwrap();
    assert!(matches!(expr, SvaExpr::And(_, _)), "Got: {:?}", expr);
}

#[test]
fn countones_different_signals_not_equiv() {
    let a = parse_sva("$countones(mask)").unwrap();
    let b = parse_sva("$countones(data)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_countones_at_nonzero_timestep() {
    let expr = parse_sva("$countones(mask)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 7);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "countones");
            assert_eq!(args[0], BoundedExpr::Var("mask@7".into()));
        }
        _ => panic!("Got: {:?}", result),
    }
}

// ══════��══════════════════════���═════════════════════════════════════════════
// SECTION 15: $isunknown(sig) — X/Z detection
// ════════════════════��══════════════════════════════��═══════════════════════

#[test]
fn parse_isunknown_system_function() {
    let expr = parse_sva("$isunknown(data_out)").unwrap();
    assert!(matches!(expr, SvaExpr::IsUnknown(_)), "Got: {:?}", expr);
}

#[test]
fn isunknown_to_string() {
    let expr = parse_sva("$isunknown(data_out)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$isunknown(data_out)");
}

#[test]
fn isunknown_roundtrip() {
    let expr = parse_sva("$isunknown(data_out)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn isunknown_structural_equiv() {
    let a = parse_sva("$isunknown(data)").unwrap();
    let b = parse_sva("$isunknown(data)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_isunknown_is_false() {
    let expr = parse_sva("$isunknown(data_out)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Bool(false),
        "In 2-state formal, $isunknown is always false. Got: {:?}", result);
}

#[test]
fn translate_isunknown_is_false_at_any_timestep() {
    let expr = parse_sva("$isunknown(x)").unwrap();
    let mut translator = SvaTranslator::new(10);
    for t in 0..10 {
        let result = translator.translate(&expr, t);
        assert_eq!(result, BoundedExpr::Bool(false),
            "$isunknown must be false at ALL timesteps. Failed at t={}. Got: {:?}", t, result);
    }
}

#[test]
fn isunknown_negated_translates_to_true() {
    let expr = parse_sva("!($isunknown(data))").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    // Not(Bool(false)) = Not(false)
    assert!(matches!(result, BoundedExpr::Not(_)),
        "!$isunknown should be Not(Bool(false)). Got: {:?}", result);
}

#[test]
fn isunknown_different_signals_not_equiv() {
    let a = parse_sva("$isunknown(data)").unwrap();
    let b = parse_sva("$isunknown(addr)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn isunknown_in_disable_iff() {
    let expr = parse_sva("disable iff ($isunknown(reset)) req |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::DisableIff { .. }), "Got: {:?}", expr);
}

// ════════��══════════════════════════════════════════════════��═══════════════
// SECTION 16: $sampled(sig) — sampled value
// ═════════��═════════════════════════════���═══════════════════════════════════

#[test]
fn parse_sampled_system_function() {
    let expr = parse_sva("$sampled(req)").unwrap();
    assert!(matches!(expr, SvaExpr::Sampled(_)), "Got: {:?}", expr);
}

#[test]
fn sampled_to_string() {
    let expr = parse_sva("$sampled(req)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$sampled(req)");
}

#[test]
fn sampled_roundtrip() {
    let expr = parse_sva("$sampled(req)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn sampled_structural_equiv() {
    let a = parse_sva("$sampled(req)").unwrap();
    let b = parse_sva("$sampled(req)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_sampled_is_identity() {
    let expr = parse_sva("$sampled(req)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 5);
    assert_eq!(result, BoundedExpr::Var("req@5".into()),
        "$sampled should be identity in synchronous formal. Got: {:?}", result);
}

#[test]
fn translate_sampled_is_identity_at_t0() {
    let expr = parse_sva("$sampled(req)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Var("req@0".into()));
}

#[test]
fn sampled_of_rose() {
    let expr = parse_sva("$sampled($rose(clk))").unwrap();
    assert!(matches!(expr, SvaExpr::Sampled(_)), "Outer should be Sampled. Got: {:?}", expr);
    if let SvaExpr::Sampled(inner) = &expr {
        assert!(matches!(inner.as_ref(), SvaExpr::Rose(_)),
            "Inner should be Rose. Got: {:?}", inner);
    }
}

#[test]
fn sampled_different_signals_not_equiv() {
    let a = parse_sva("$sampled(req)").unwrap();
    let b = parse_sva("$sampled(ack)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn sampled_in_implication() {
    let expr = parse_sva("$sampled(req) |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }), "Got: {:?}", expr);
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("$sampled(req)"), "Got: {}", text);
}

// ══════��════════════════════════════════════════════════════════════════════
// SECTION 17: $bits(sig) — bit width
// ═══════════════════��═════════════════════════════════════���═════════════════

#[test]
fn parse_bits_system_function() {
    let expr = parse_sva("$bits(data)").unwrap();
    assert!(matches!(expr, SvaExpr::Bits(_)), "Got: {:?}", expr);
}

#[test]
fn bits_to_string() {
    let expr = parse_sva("$bits(data)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$bits(data)");
}

#[test]
fn bits_roundtrip() {
    let expr = parse_sva("$bits(data)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn bits_structural_equiv() {
    let a = parse_sva("$bits(data)").unwrap();
    let b = parse_sva("$bits(data)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_bits_produces_apply() {
    let expr = parse_sva("$bits(data)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "bits");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], BoundedExpr::Var("data@0".into()));
        }
        _ => panic!("$bits should translate to Apply. Got: {:?}", result),
    }
}

#[test]
fn bits_in_comparison() {
    let expr = parse_sva("$bits(data) == 32").unwrap();
    match &expr {
        SvaExpr::Eq(left, right) => {
            assert!(matches!(left.as_ref(), SvaExpr::Bits(_)),
                "LHS should be Bits. Got: {:?}", left);
            assert!(matches!(right.as_ref(), SvaExpr::Const(32, 32)),
                "RHS should be Const(32). Got: {:?}", right);
        }
        _ => panic!("Expected Eq(Bits, Const). Got: {:?}", expr),
    }
}

#[test]
fn bits_different_signals_not_equiv() {
    let a = parse_sva("$bits(data)").unwrap();
    let b = parse_sva("$bits(addr)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn bits_greater_than_comparison() {
    let expr = parse_sva("$bits(data) > 8").unwrap();
    assert!(matches!(expr, SvaExpr::GreaterThan(_, _)), "Got: {:?}", expr);
}

#[test]
fn translate_bits_at_nonzero_timestep() {
    let expr = parse_sva("$bits(data)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 4);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "bits");
            assert_eq!(args[0], BoundedExpr::Var("data@4".into()));
        }
        _ => panic!("Got: {:?}", result),
    }
}

#[test]
fn bits_in_clog2_comparison() {
    let expr = parse_sva("$bits(addr) == $clog2(1024)").unwrap();
    assert!(matches!(expr, SvaExpr::Eq(_, _)), "Got: {:?}", expr);
    if let SvaExpr::Eq(l, r) = &expr {
        assert!(matches!(l.as_ref(), SvaExpr::Bits(_)), "LHS: {:?}", l);
        assert!(matches!(r.as_ref(), SvaExpr::Clog2(_)), "RHS: {:?}", r);
    }
}

// ════════════════════════════════════════════════════════════════════��══════
// SECTION 18: $clog2(val) — ceiling log2
// ══════════════════════════���═════════════════════════════════��══════════════

#[test]
fn parse_clog2_system_function() {
    let expr = parse_sva("$clog2(256)").unwrap();
    assert!(matches!(expr, SvaExpr::Clog2(_)), "Got: {:?}", expr);
}

#[test]
fn clog2_to_string() {
    let expr = parse_sva("$clog2(256)").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "$clog2(32'd256)");
}

#[test]
fn clog2_roundtrip() {
    let expr = parse_sva("$clog2(depth)").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn clog2_structural_equiv() {
    let a = parse_sva("$clog2(depth)").unwrap();
    let b = parse_sva("$clog2(depth)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_clog2_constant_folds() {
    let mut translator = SvaTranslator::new(5);
    // $clog2(8) = 3
    let expr = parse_sva("$clog2(8)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(3), "$clog2(8) should fold to 3. Got: {:?}", result);

    // $clog2(16) = 4
    let expr = parse_sva("$clog2(16)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(4), "$clog2(16) should fold to 4. Got: {:?}", result);

    // $clog2(1) = 0
    let expr = parse_sva("$clog2(1)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(0), "$clog2(1) should fold to 0. Got: {:?}", result);
}

#[test]
fn translate_clog2_zero_is_zero() {
    let expr = parse_sva("$clog2(0)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(0), "$clog2(0) should be 0 (IEEE edge case). Got: {:?}", result);
}

#[test]
fn translate_clog2_non_power_of_two() {
    let mut translator = SvaTranslator::new(5);
    // $clog2(5) = 3 (ceil)
    let expr = parse_sva("$clog2(5)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(3), "$clog2(5) should fold to 3. Got: {:?}", result);
}

#[test]
fn translate_clog2_variable_produces_apply() {
    let expr = parse_sva("$clog2(depth)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    match &result {
        BoundedExpr::Apply { name, args } => {
            assert_eq!(name, "clog2");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], BoundedExpr::Var("depth@0".into()));
        }
        _ => panic!("$clog2(variable) should translate to Apply. Got: {:?}", result),
    }
}

#[test]
fn translate_clog2_large_power() {
    let mut translator = SvaTranslator::new(5);
    // $clog2(1024) = 10
    let expr = parse_sva("$clog2(1024)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(10), "$clog2(1024) should be 10. Got: {:?}", result);
}

#[test]
fn translate_clog2_two() {
    let mut translator = SvaTranslator::new(5);
    // $clog2(2) = 1
    let expr = parse_sva("$clog2(2)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(1), "$clog2(2) should be 1. Got: {:?}", result);
}

#[test]
fn translate_clog2_three() {
    let mut translator = SvaTranslator::new(5);
    // $clog2(3) = 2 (ceiling)
    let expr = parse_sva("$clog2(3)").unwrap();
    let result = translator.translate(&expr, 0);
    assert_eq!(result, BoundedExpr::Int(2), "$clog2(3) should be 2. Got: {:?}", result);
}

#[test]
fn clog2_different_args_not_equiv() {
    let a = parse_sva("$clog2(8)").unwrap();
    let b = parse_sva("$clog2(16)").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn clog2_in_equality_with_bits() {
    // Real use case: $bits(addr) == $clog2(DEPTH)
    let expr = parse_sva("$bits(addr) == $clog2(256)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    // LHS: Apply("bits", [Var("addr@0")]), RHS: Int(8) (folded)
    match &result {
        BoundedExpr::Eq(_, rhs) => {
            assert_eq!(rhs.as_ref(), &BoundedExpr::Int(8),
                "$clog2(256) should fold to 8 inside comparison. Got: {:?}", rhs);
        }
        _ => panic!("Expected Eq. Got: {:?}", result),
    }
}

#[test]
fn all_seven_system_functions_parse_independently() {
    // Verify the parser doesn't confuse any of the 7 new system functions
    let tests = vec![
        ("$onehot0(x)", "OneHot0"),
        ("$onehot(x)", "OneHot"),
        ("$countones(x)", "CountOnes"),
        ("$isunknown(x)", "IsUnknown"),
        ("$sampled(x)", "Sampled"),
        ("$bits(x)", "Bits"),
        ("$clog2(x)", "Clog2"),
    ];
    for (input, expected_name) in &tests {
        let expr = parse_sva(input).unwrap();
        let name = format!("{:?}", expr);
        assert!(name.starts_with(expected_name),
            "Input '{}' should parse to {}. Got: {}", input, expected_name, name);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 19: [->N] — goto repetition (non-consecutive)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_goto_repetition() {
    let expr = parse_sva("ack[->1]").unwrap();
    match &expr {
        SvaExpr::GotoRepetition { body, count } => {
            assert!(matches!(body.as_ref(), SvaExpr::Signal(s) if s == "ack"),
                "Body should be Signal(ack). Got: {:?}", body);
            assert_eq!(*count, 1);
        }
        _ => panic!("Expected GotoRepetition. Got: {:?}", expr),
    }
}

#[test]
fn parse_goto_repetition_count_3() {
    let expr = parse_sva("ack[->3]").unwrap();
    match &expr {
        SvaExpr::GotoRepetition { count, .. } => assert_eq!(*count, 3),
        _ => panic!("Expected GotoRepetition. Got: {:?}", expr),
    }
}

#[test]
fn goto_repetition_to_string() {
    let expr = parse_sva("ack[->1]").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "ack[->1]");
}

#[test]
fn goto_repetition_to_string_count_3() {
    let expr = parse_sva("done[->3]").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "done[->3]");
}

#[test]
fn goto_repetition_roundtrip() {
    let expr = parse_sva("ack[->1]").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed));
}

#[test]
fn goto_repetition_structural_equiv() {
    let a = parse_sva("ack[->1]").unwrap();
    let b = parse_sva("ack[->1]").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn goto_repetition_structural_equiv_different_count() {
    let a = parse_sva("ack[->1]").unwrap();
    let b = parse_sva("ack[->2]").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b),
        "Different goto counts must NOT be structurally equivalent");
}

#[test]
fn goto_repetition_structural_equiv_different_signal() {
    let a = parse_sva("ack[->1]").unwrap();
    let b = parse_sva("done[->1]").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_goto_repetition_1_is_disjunction() {
    let expr = parse_sva("sig[->1]").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    // sig[->1] at bound=5 → sig@1 ∨ sig@2 ∨ sig@3 ∨ sig@4 ∨ sig@5
    let leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(leaves, 5, "sig[->1] at bound=5 should produce 5 Or leaves. Got: {}", leaves);
}

#[test]
fn translate_goto_repetition_gt1_produces_combinatorial_disjunction() {
    // sig[->3] at bound=5: C(5,3)=10 combinations, each a conjunction of 3 vars
    let expr = parse_sva("sig[->3]").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[->3] must NOT be Unsupported. Got: {:?}", result);
    let leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(leaves, 10, "C(5,3)=10 Or-branches expected. Got: {}", leaves);
}

#[test]
fn goto_repetition_1_references_signal_at_multiple_timesteps() {
    let expr = parse_sva("ack[->1]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    let ack_timesteps: Vec<&String> = vars.iter().filter(|v| v.starts_with("ack@")).collect();
    assert!(ack_timesteps.len() >= 2,
        "[->1] must reference ack at multiple timesteps. Got: {:?}", ack_timesteps);
}

#[test]
fn goto_does_not_conflict_with_consecutive_repetition() {
    // [*] and [->] must coexist
    let star = parse_sva("sig[*3]").unwrap();
    let goto = parse_sva("sig[->3]").unwrap();
    assert!(matches!(star, SvaExpr::Repetition { .. }), "Got: {:?}", star);
    assert!(matches!(goto, SvaExpr::GotoRepetition { .. }), "Got: {:?}", goto);
    assert!(!sva_exprs_structurally_equivalent(&star, &goto));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 20: [=N] / [=min:max] — non-consecutive repetition
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_nonconsec_repetition_exact() {
    let expr = parse_sva("ack[=3]").unwrap();
    match &expr {
        SvaExpr::NonConsecRepetition { body, min, max } => {
            assert!(matches!(body.as_ref(), SvaExpr::Signal(s) if s == "ack"));
            assert_eq!(*min, 3);
            assert_eq!(*max, Some(3));
        }
        _ => panic!("Expected NonConsecRepetition. Got: {:?}", expr),
    }
}

#[test]
fn parse_nonconsec_repetition_range() {
    let expr = parse_sva("ack[=1:3]").unwrap();
    match &expr {
        SvaExpr::NonConsecRepetition { min, max, .. } => {
            assert_eq!(*min, 1);
            assert_eq!(*max, Some(3));
        }
        _ => panic!("Expected NonConsecRepetition. Got: {:?}", expr),
    }
}

#[test]
fn parse_nonconsec_repetition_unbounded() {
    let expr = parse_sva("ack[=2:$]").unwrap();
    match &expr {
        SvaExpr::NonConsecRepetition { min, max, .. } => {
            assert_eq!(*min, 2);
            assert_eq!(*max, None);
        }
        _ => panic!("Expected NonConsecRepetition. Got: {:?}", expr),
    }
}

#[test]
fn nonconsec_to_string_exact() {
    let expr = parse_sva("ack[=3]").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "ack[=3]");
}

#[test]
fn nonconsec_to_string_range() {
    let expr = parse_sva("ack[=1:3]").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "ack[=1:3]");
}

#[test]
fn nonconsec_to_string_unbounded() {
    let expr = parse_sva("ack[=2:$]").unwrap();
    assert_eq!(sva_expr_to_string(&expr), "ack[=2:$]");
}

#[test]
fn nonconsec_roundtrip() {
    for input in &["ack[=3]", "ack[=1:3]", "ack[=2:$]"] {
        let expr = parse_sva(input).unwrap();
        let text = sva_expr_to_string(&expr);
        let reparsed = parse_sva(&text).unwrap();
        assert!(sva_exprs_structurally_equivalent(&expr, &reparsed),
            "Roundtrip failed for '{}'. Rendered: '{}'", input, text);
    }
}

#[test]
fn nonconsec_structural_equiv() {
    let a = parse_sva("ack[=3]").unwrap();
    let b = parse_sva("ack[=3]").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn nonconsec_structural_equiv_different_count() {
    let a = parse_sva("ack[=3]").unwrap();
    let b = parse_sva("ack[=2]").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn nonconsec_structural_equiv_different_signal() {
    let a = parse_sva("ack[=3]").unwrap();
    let b = parse_sva("done[=3]").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_nonconsec_exact_produces_combinatorial_disjunction() {
    // ack[=3] at bound=5: C(5,3)=10 combinations, each with 3 true + 2 Not
    let expr = parse_sva("ack[=3]").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[=3] must NOT be Unsupported. Got: {:?}", result);
    let leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(leaves, 10, "C(5,3)=10 Or-branches expected. Got: {}", leaves);
}

#[test]
fn nonconsec_does_not_conflict_with_goto_or_consecutive() {
    let star = parse_sva("sig[*3]").unwrap();
    let goto = parse_sva("sig[->3]").unwrap();
    let nonconsec = parse_sva("sig[=3]").unwrap();
    assert!(matches!(star, SvaExpr::Repetition { .. }));
    assert!(matches!(goto, SvaExpr::GotoRepetition { .. }));
    assert!(matches!(nonconsec, SvaExpr::NonConsecRepetition { .. }));
    // All three are distinct
    assert!(!sva_exprs_structurally_equivalent(&star, &goto));
    assert!(!sva_exprs_structurally_equivalent(&star, &nonconsec));
    assert!(!sva_exprs_structurally_equivalent(&goto, &nonconsec));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 21: accept_on(cond) body — property passes if cond true
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_accept_on() {
    let expr = parse_sva("accept_on(flush) req |-> ack").unwrap();
    match &expr {
        SvaExpr::AcceptOn { condition, body } => {
            assert!(matches!(condition.as_ref(), SvaExpr::Signal(s) if s == "flush"),
                "Condition should be flush. Got: {:?}", condition);
            assert!(matches!(body.as_ref(), SvaExpr::Implication { .. }),
                "Body should be implication. Got: {:?}", body);
        }
        _ => panic!("Expected AcceptOn. Got: {:?}", expr),
    }
}

#[test]
fn accept_on_to_string() {
    let expr = parse_sva("accept_on(flush) req").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "accept_on(flush) req");
}

#[test]
fn accept_on_roundtrip() {
    let expr = parse_sva("accept_on(flush) req |-> ack").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed),
        "Roundtrip failed. Rendered: '{}'", text);
}

#[test]
fn accept_on_structural_equiv() {
    let a = parse_sva("accept_on(flush) req").unwrap();
    let b = parse_sva("accept_on(flush) req").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn accept_on_structural_equiv_different_condition() {
    let a = parse_sva("accept_on(flush) req").unwrap();
    let b = parse_sva("accept_on(abort) req").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn accept_on_structural_equiv_different_body() {
    let a = parse_sva("accept_on(flush) req").unwrap();
    let b = parse_sva("accept_on(flush) ack").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_accept_on_is_or() {
    let expr = parse_sva("accept_on(flush) valid").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 2);
    // accept_on(C) P ≡ C ∨ P
    match &result {
        BoundedExpr::Or(left, right) => {
            assert_eq!(left.as_ref(), &BoundedExpr::Var("flush@2".into()),
                "Left should be flush@2. Got: {:?}", left);
            assert_eq!(right.as_ref(), &BoundedExpr::Var("valid@2".into()),
                "Right should be valid@2. Got: {:?}", right);
        }
        _ => panic!("accept_on should translate to Or. Got: {:?}", result),
    }
}

#[test]
fn accept_on_with_disable_iff() {
    let expr = parse_sva("accept_on(flush) disable iff (reset) req |-> ack").unwrap();
    assert!(matches!(expr, SvaExpr::AcceptOn { .. }),
        "Outer should be AcceptOn. Got: {:?}", expr);
    if let SvaExpr::AcceptOn { body, .. } = &expr {
        assert!(matches!(body.as_ref(), SvaExpr::DisableIff { .. }),
            "Inner should be DisableIff. Got: {:?}", body);
    }
}

#[test]
fn accept_on_with_complex_body() {
    let expr = parse_sva("accept_on(exception) $onehot(state) |-> s_eventually(done)").unwrap();
    assert!(matches!(expr, SvaExpr::AcceptOn { .. }), "Got: {:?}", expr);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 22: reject_on(cond) body — property fails if cond true
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_reject_on() {
    let expr = parse_sva("reject_on(error) req |-> ack").unwrap();
    match &expr {
        SvaExpr::RejectOn { condition, body } => {
            assert!(matches!(condition.as_ref(), SvaExpr::Signal(s) if s == "error"),
                "Condition should be error. Got: {:?}", condition);
            assert!(matches!(body.as_ref(), SvaExpr::Implication { .. }),
                "Body should be implication. Got: {:?}", body);
        }
        _ => panic!("Expected RejectOn. Got: {:?}", expr),
    }
}

#[test]
fn reject_on_to_string() {
    let expr = parse_sva("reject_on(error) valid").unwrap();
    let text = sva_expr_to_string(&expr);
    assert_eq!(text, "reject_on(error) valid");
}

#[test]
fn reject_on_roundtrip() {
    let expr = parse_sva("reject_on(error) req |-> ack").unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed),
        "Roundtrip failed. Rendered: '{}'", text);
}

#[test]
fn reject_on_structural_equiv() {
    let a = parse_sva("reject_on(error) req").unwrap();
    let b = parse_sva("reject_on(error) req").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn reject_on_structural_equiv_different_condition() {
    let a = parse_sva("reject_on(error) req").unwrap();
    let b = parse_sva("reject_on(fault) req").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn translate_reject_on_is_and_not() {
    let expr = parse_sva("reject_on(error) valid").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 2);
    // reject_on(C) P ≡ ¬C ∧ P
    match &result {
        BoundedExpr::And(left, right) => {
            // left should be Not(Var("error@2"))
            match left.as_ref() {
                BoundedExpr::Not(inner) => {
                    assert_eq!(inner.as_ref(), &BoundedExpr::Var("error@2".into()),
                        "Inner of Not should be error@2. Got: {:?}", inner);
                }
                _ => panic!("Left should be Not. Got: {:?}", left),
            }
            assert_eq!(right.as_ref(), &BoundedExpr::Var("valid@2".into()),
                "Right should be valid@2. Got: {:?}", right);
        }
        _ => panic!("reject_on should translate to And(Not, P). Got: {:?}", result),
    }
}

#[test]
fn reject_on_with_complex_body() {
    let expr = parse_sva("reject_on(overflow) $countones(mask) >= 2").unwrap();
    assert!(matches!(expr, SvaExpr::RejectOn { .. }), "Got: {:?}", expr);
}

#[test]
fn accept_on_and_reject_on_are_not_equiv() {
    let a = parse_sva("accept_on(x) y").unwrap();
    let b = parse_sva("reject_on(x) y").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b),
        "accept_on and reject_on are DIFFERENT operators");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 23: Composition & Cross-Cutting Edge Cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn onehot0_nested_in_implication_roundtrip() {
    let input = "$onehot0(grant) |-> $stable(data)";
    let expr = parse_sva(input).unwrap();
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(sva_exprs_structurally_equivalent(&expr, &reparsed),
        "Complex nested expression must roundtrip. Rendered: '{}'", text);
}

#[test]
fn goto_repetition_in_sequence_delay() {
    // req ##1 ack[->1] — sequence concat with goto
    let expr = parse_sva("req ##1 ack[->1]").unwrap();
    assert!(matches!(expr, SvaExpr::Implication { .. }),
        "Sequence concat should produce implication. Got: {:?}", expr);
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("[->1]"), "Must preserve goto repetition in sequence. Got: {}", text);
}

#[test]
fn isunknown_negated_translates_correctly() {
    let expr = parse_sva("!($isunknown(data))").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    // Not(Bool(false)) — semantically true, but structurally Not(false)
    match &result {
        BoundedExpr::Not(inner) => {
            assert_eq!(inner.as_ref(), &BoundedExpr::Bool(false));
        }
        _ => panic!("Got: {:?}", result),
    }
}

#[test]
fn clog2_in_bits_comparison_folds_correctly() {
    let expr = parse_sva("$bits(addr) == $clog2(1024)").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 0);
    // LHS: Apply("bits", [Var("addr@0")]), RHS: Int(10)
    match &result {
        BoundedExpr::Eq(lhs, rhs) => {
            match lhs.as_ref() {
                BoundedExpr::Apply { name, .. } => assert_eq!(name, "bits"),
                _ => panic!("LHS should be Apply(bits). Got: {:?}", lhs),
            }
            assert_eq!(rhs.as_ref(), &BoundedExpr::Int(10),
                "$clog2(1024) must fold to 10. Got: {:?}", rhs);
        }
        _ => panic!("Expected Eq. Got: {:?}", result),
    }
}

#[test]
fn sampled_of_rose_translates_correctly() {
    let expr = parse_sva("$sampled($rose(clk))").unwrap();
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate(&expr, 3);
    // $sampled is identity → $rose(clk) at t=3 → And(clk@3, Not(clk@2))
    match &result {
        BoundedExpr::And(left, right) => {
            assert_eq!(left.as_ref(), &BoundedExpr::Var("clk@3".into()));
            match right.as_ref() {
                BoundedExpr::Not(inner) => {
                    assert_eq!(inner.as_ref(), &BoundedExpr::Var("clk@2".into()));
                }
                _ => panic!("Expected Not(clk@2). Got: {:?}", right),
            }
        }
        _ => panic!("$sampled($rose(clk))@3 should be And(clk@3, Not(clk@2)). Got: {:?}", result),
    }
}

#[test]
fn multiple_system_functions_in_conjunction() {
    let expr = parse_sva("$onehot(state) && $onehot0(grant)").unwrap();
    assert!(matches!(expr, SvaExpr::And(_, _)), "Got: {:?}", expr);
    if let SvaExpr::And(l, r) = &expr {
        assert!(matches!(l.as_ref(), SvaExpr::OneHot(_)));
        assert!(matches!(r.as_ref(), SvaExpr::OneHot0(_)));
    }
}

#[test]
fn system_function_in_disable_iff() {
    let expr = parse_sva("disable iff ($isunknown(reset)) $onehot(state)").unwrap();
    assert!(matches!(expr, SvaExpr::DisableIff { .. }), "Got: {:?}", expr);
}

#[test]
fn all_new_constructs_do_not_break_existing_parser() {
    // Verify existing constructs still parse after adding new ones
    let existing_cases = vec![
        "$rose(clk)",
        "$fell(reset)",
        "$stable(data)",
        "$changed(addr)",
        "$past(sig, 2)",
        "req |-> ack",
        "req |=> ack",
        "req ##1 ack",
        "sig[*3]",
        "s_eventually(done)",
        "s_always(valid)",
        "disable iff (reset) req |-> ack",
        "nexttime(done)",
        "first_match(req ##[1:5] ack)",
        "valid throughout (req ##1 ack)",
        "req != ack",
        "count < 10",
        "mode ? req : idle",
    ];
    for case in &existing_cases {
        let result = parse_sva(case);
        assert!(result.is_ok(), "Existing parse case '{}' MUST still work. Error: {:?}", case, result.err());
    }
}

#[test]
fn property_level_translation_with_new_constructs() {
    // Verify translate_property works with new system functions
    let expr = parse_sva("$onehot0(grant)").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate_property(&expr);
    // Should be conjunction: onehot0(grant@0) ∧ onehot0(grant@1) ∧ onehot0(grant@2)
    let leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&result.expr);
    assert_eq!(leaves, 3, "Property over 3 timesteps = 3 And-leaves. Got: {}", leaves);
}

#[test]
fn accept_on_preserves_inner_property_structure() {
    let expr = parse_sva("accept_on(flush) $onehot(state) |-> s_eventually(done)").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    // Or(flush@0, Implies(Apply("onehot", [state@0]), Or-tree))
    assert!(matches!(result, BoundedExpr::Or(_, _)),
        "accept_on must produce Or. Got: {:?}", result);
}

#[test]
fn reject_on_preserves_inner_property_structure() {
    let expr = parse_sva("reject_on(error) $onehot0(grant) |-> ack").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    // And(Not(error@0), Implies(Apply("onehot0", [grant@0]), ack@0))
    assert!(matches!(result, BoundedExpr::And(_, _)),
        "reject_on must produce And. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 24: [->N] Goto Repetition — full combinatorial translation
// ═══════════════════════════════════════════════════════════════════════════

fn count_not_nodes(expr: &BoundedExpr) -> usize {
    match expr {
        BoundedExpr::Not(inner) => 1 + count_not_nodes(inner),
        BoundedExpr::And(l, r) | BoundedExpr::Or(l, r)
        | BoundedExpr::Implies(l, r) | BoundedExpr::Eq(l, r) => {
            count_not_nodes(l) + count_not_nodes(r)
        }
        _ => 0,
    }
}

#[test]
fn translate_goto_repetition_2_bound_3() {
    // sig[->2] at bound=3: C(3,2)=3 combinations: {1,2}, {1,3}, {2,3}
    let expr = parse_sva("sig[->2]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[->2] must NOT be Unsupported. Got: {:?}", result);
    let or_leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(or_leaves, 3, "C(3,2)=3 Or-branches expected. Got: {}", or_leaves);
}

#[test]
fn translate_goto_repetition_2_references_correct_vars() {
    let expr = parse_sva("sig[->2]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    let vars = collect_bounded_vars(&result);
    // Should reference sig@1, sig@2, sig@3 (all positions in window)
    assert!(vars.iter().any(|v| v == "sig@1"), "Must reference sig@1. Vars: {:?}", vars);
    assert!(vars.iter().any(|v| v == "sig@2"), "Must reference sig@2. Vars: {:?}", vars);
    assert!(vars.iter().any(|v| v == "sig@3"), "Must reference sig@3. Vars: {:?}", vars);
}

#[test]
fn translate_goto_repetition_exceeds_bound_is_false() {
    // sig[->6] at bound=3: impossible — can't fit 6 matches in 3 timesteps
    let expr = parse_sva("sig[->6]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(matches!(result, BoundedExpr::Bool(false)),
        "[->6] at bound=3 must be Bool(false). Got: {:?}", result);
}

#[test]
fn translate_goto_repetition_count_equals_bound() {
    // sig[->3] at bound=3: C(3,3)=1 combination: {1,2,3} — only one way
    let expr = parse_sva("sig[->3]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[->3] at bound=3 must NOT be Unsupported. Got: {:?}", result);
    let or_leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(or_leaves, 1, "C(3,3)=1 single conjunction expected. Got: {}", or_leaves);
}

#[test]
fn translate_goto_repetition_no_not_nodes() {
    // GotoRepetition should NOT produce Not nodes — only positive assertions
    let expr = parse_sva("sig[->2]").unwrap();
    let mut translator = SvaTranslator::new(4);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[->2] at bound=4 must NOT be Unsupported. Got: {:?}", result);
    let nots = count_not_nodes(&result);
    assert_eq!(nots, 0, "GotoRepetition must NOT contain Not nodes. Got {} Not nodes", nots);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 25: [=N] Non-Consecutive Repetition — full combinatorial translation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn translate_nonconsec_exact_2_bound_3() {
    // ack[=2] at bound=3: C(3,2)=3 combinations, each with 2 true + 1 Not
    let expr = parse_sva("ack[=2]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[=2] must NOT be Unsupported. Got: {:?}", result);
    let or_leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(or_leaves, 3, "C(3,2)=3 Or-branches expected. Got: {}", or_leaves);
}

#[test]
fn translate_nonconsec_range_1_to_2_bound_3() {
    // ack[=1:2] at bound=3: count=1 has C(3,1)=3, count=2 has C(3,2)=3, total=6
    let expr = parse_sva("ack[=1:2]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[=1:2] must NOT be Unsupported. Got: {:?}", result);
    let or_leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(or_leaves, 6, "C(3,1)+C(3,2)=3+3=6 Or-branches expected. Got: {}", or_leaves);
}

#[test]
fn translate_nonconsec_unbounded_max_bound_3() {
    // ack[=1:$] at bound=3: count=1→C(3,1)=3, count=2→C(3,2)=3, count=3→C(3,3)=1, total=7
    let expr = parse_sva("ack[=1:$]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[=1:$] must NOT be Unsupported. Got: {:?}", result);
    let or_leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(or_leaves, 7, "C(3,1)+C(3,2)+C(3,3)=3+3+1=7 Or-branches expected. Got: {}", or_leaves);
}

#[test]
fn translate_nonconsec_min_exceeds_bound_is_false() {
    // ack[=5] at bound=3: impossible — can't have 5 true cycles in 3 timesteps
    let expr = parse_sva("ack[=5]").unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate(&expr, 0);
    assert!(matches!(result, BoundedExpr::Bool(false)),
        "[=5] at bound=3 must be Bool(false). Got: {:?}", result);
}

#[test]
fn translate_nonconsec_includes_negations() {
    // ack[=1] at bound=2: 2 combinations: {ack@1, !ack@2} or {!ack@1, ack@2}
    // Must contain Not nodes (unlike GotoRepetition)
    let expr = parse_sva("ack[=1]").unwrap();
    let mut translator = SvaTranslator::new(2);
    let result = translator.translate(&expr, 0);
    let nots = count_not_nodes(&result);
    assert!(nots > 0, "NonConsecRepetition must contain Not nodes for exact-count. Got 0. Result: {:?}", result);
    // Each of the 2 combinations has 1 Not → 2 Nots total
    assert_eq!(nots, 2, "ack[=1] at bound=2 should have 2 Not nodes. Got: {}", nots);
}

#[test]
fn translate_nonconsec_exact_0_bound_2() {
    // ack[=0] at bound=2: zero matches → !ack@1 && !ack@2
    let expr = parse_sva("ack[=0]").unwrap();
    let mut translator = SvaTranslator::new(2);
    let result = translator.translate(&expr, 0);
    assert!(!matches!(result, BoundedExpr::Unsupported(_)),
        "[=0] must NOT be Unsupported. Got: {:?}", result);
    let nots = count_not_nodes(&result);
    assert_eq!(nots, 2, "ack[=0] at bound=2: all positions negated → 2 Not nodes. Got: {}", nots);
    let or_leaves = logicaffeine_compile::codegen_sva::sva_to_verify::count_or_leaves(&result);
    assert_eq!(or_leaves, 1, "ack[=0] at bound=2: C(2,0)=1 single conjunction. Got: {}", or_leaves);
}
