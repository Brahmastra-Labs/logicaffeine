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

use logicaffeine_compile::codegen_sva::sva_model::{parse_sva, sva_expr_to_string, SvaExpr};
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
    // Throughout: signal must hold at every timestep of the sequence
    assert!(matches!(result, BoundedExpr::And(_, _)),
        "Throughout should produce conjunction. Got: {:?}", result);
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
