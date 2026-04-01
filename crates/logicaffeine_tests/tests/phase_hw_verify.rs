//! Sprint C: Verification IR Extensions — Bitvectors & BMC
//!
//! Tests for hardware verification features. Tests that need Z3 are gated
//! behind the `verification` feature flag.
//!
//! Run WITHOUT Z3: cargo test --test phase_hw_verify -- --skip e2e
//! Run WITH Z3:    cargo test --features verification --test phase_hw_verify -- --skip e2e

use logicaffeine_verify::{BitVecOp, VerifyExpr, VerifyOp, VerifyType};
#[cfg(feature = "verification")]
use logicaffeine_verify::VerificationSession;

// ═══════════════════════════════════════════════════════════════════════════
// COMPILE-TIME: existing tests still pass after our AST changes
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hw_changes_do_not_break_basic_compilation() {
    use logicaffeine_compile::compile::compile_to_rust;
    let source = "## Main\nLet x be 42.\nShow x.";
    let output = compile_to_rust(source).unwrap();
    assert!(output.contains("42") || output.contains("let "),
        "Compiled Rust must contain the literal or variable binding. Got length: {}", output.len());
}

#[test]
fn hw_changes_do_not_break_refinement_syntax() {
    use logicaffeine_compile::compile::compile_to_rust;
    let source = "## Main\nLet x: Int where it > 0 be 10.\nShow x.";
    let output = compile_to_rust(source).unwrap();
    assert!(output.contains("10") || output.contains("let "),
        "Refinement type program must compile to Rust with the value. Got length: {}", output.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// PATTERN: BMC as LOGOS refinement types (no Z3 needed for parse check)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bmc_counter_pattern_compiles() {
    use logicaffeine_compile::compile::compile_to_rust;
    let source = r#"## Main
Let counter be 0.
Let next be counter + 1.
Show next.
"#;
    let output = compile_to_rust(source).unwrap();
    assert!(output.contains("counter") || output.contains("next"),
        "Counter pattern must reference counter/next in output. Got length: {}", output.len());
}

#[test]
fn mutex_pattern_compiles() {
    use logicaffeine_compile::compile::compile_to_rust;
    let source = r#"## Main
Let grant_a be 1.
Let grant_b be 0.
Let both be grant_a + grant_b.
If both is greater than 1:
    Show "MUTEX VIOLATION".
Otherwise:
    Show "OK".
"#;
    let output = compile_to_rust(source).unwrap();
    assert!(output.contains("MUTEX VIOLATION") || output.contains("grant"),
        "Mutex pattern must contain the violation string or variable names. Got length: {}", output.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// Z3 VERIFICATION (behind feature flag — tests skip when Z3 unavailable)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[cfg(feature = "verification")]
fn z3_refinement_valid_after_hw_changes() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = "## Main\nLet x: Int where it > 0 be 10.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Z3 should verify 10 > 0: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn z3_refinement_invalid_after_hw_changes() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = "## Main\nLet x: Int where it > 0 be -5.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Z3 should reject -5 > 0");
}

#[test]
#[cfg(feature = "verification")]
fn z3_mutex_valid() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = r#"## Main
Let grant_a: Int where it >= 0 and it <= 1 be 1.
Let grant_b: Int where it >= 0 and it <= 1 be 0.
Let sum: Int where it <= 1 be grant_a + grant_b.
Show sum.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "1+0=1 satisfies sum<=1: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn z3_mutex_violation() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = r#"## Main
Let grant_a: Int where it >= 0 and it <= 1 be 1.
Let grant_b: Int where it >= 0 and it <= 1 be 1.
Let sum: Int where it <= 1 be grant_a + grant_b.
Show sum.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "1+1=2 violates sum<=1 (mutex violation)");
}

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR IR (compile-time, no Z3 needed)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bitvector_type_variant_exists() {
    use logicaffeine_verify::VerifyType;
    let bv8 = VerifyType::BitVector(8);
    match bv8 {
        VerifyType::BitVector(w) => assert_eq!(w, 8),
        _ => panic!("BitVector must be a distinct VerifyType variant"),
    }
}

#[test]
fn bitvector_const_expr_exists() {
    use logicaffeine_verify::VerifyExpr;
    let bv = VerifyExpr::BitVecConst { width: 8, value: 0xFF };
    match bv {
        VerifyExpr::BitVecConst { width, value } => {
            assert_eq!(width, 8);
            assert_eq!(value, 0xFF);
        }
        _ => panic!("BitVecConst must be a distinct VerifyExpr variant"),
    }
}

#[test]
fn bitvector_binary_op_exists() {
    use logicaffeine_verify::{BitVecOp, VerifyExpr};
    let a = VerifyExpr::BitVecConst { width: 8, value: 0xFF };
    let b = VerifyExpr::BitVecConst { width: 8, value: 0x0F };
    let and = VerifyExpr::BitVecBinary {
        op: BitVecOp::And,
        left: Box::new(a),
        right: Box::new(b),
    };
    match and {
        VerifyExpr::BitVecBinary { op: BitVecOp::And, .. } => {}
        _ => panic!("BitVecBinary with And op must construct"),
    }
}

#[test]
fn array_type_variant_exists() {
    use logicaffeine_verify::VerifyType;
    let arr = VerifyType::Array(
        Box::new(VerifyType::BitVector(8)),
        Box::new(VerifyType::BitVector(32)),
    );
    match arr {
        VerifyType::Array(idx, elem) => {
            assert!(matches!(*idx, VerifyType::BitVector(8)));
            assert!(matches!(*elem, VerifyType::BitVector(32)));
        }
        _ => panic!("Array must be a distinct VerifyType variant"),
    }
}

#[test]
fn iff_expr_variant_exists() {
    use logicaffeine_verify::VerifyExpr;
    let a = VerifyExpr::Bool(true);
    let b = VerifyExpr::Bool(false);
    let iff = VerifyExpr::Iff(Box::new(a), Box::new(b));
    assert!(matches!(iff, VerifyExpr::Iff(_, _)), "Iff must be a distinct variant");
}

#[test]
fn select_store_expr_variants_exist() {
    use logicaffeine_verify::VerifyExpr;
    let arr = VerifyExpr::Var("mem".into());
    let idx = VerifyExpr::BitVecConst { width: 8, value: 0 };
    let val = VerifyExpr::BitVecConst { width: 32, value: 42 };

    let sel = VerifyExpr::Select {
        array: Box::new(arr.clone()),
        index: Box::new(idx.clone()),
    };
    assert!(matches!(sel, VerifyExpr::Select { .. }));

    let sto = VerifyExpr::Store {
        array: Box::new(arr),
        index: Box::new(idx),
        value: Box::new(val),
    };
    assert!(matches!(sto, VerifyExpr::Store { .. }));
}

// ═══════════════════════════════════════════════════════════════════════════
// Z3 BITVECTOR ENCODING (feature-gated)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[cfg(feature = "verification")]
fn z3_bitvector_declaration_and_assumption_works() {
    let mut session = VerificationSession::new();
    session.declare("sig", VerifyType::BitVector(8));
    session.assume(&VerifyExpr::eq(VerifyExpr::var("sig"), VerifyExpr::bv_const(8, 42)));
    let check = VerifyExpr::eq(VerifyExpr::var("sig"), VerifyExpr::bv_const(8, 42));
    assert!(session.verify(&check).is_ok(),
        "BV declaration + assumption must allow verification");
}

#[test]
#[cfg(feature = "verification")]
fn z3_iff_encoding_works() {
    use logicaffeine_verify::{VerificationSession, VerifyExpr};
    let session = VerificationSession::new();
    // true ↔ true should be valid
    let expr = VerifyExpr::iff(VerifyExpr::Bool(true), VerifyExpr::Bool(true));
    let result = session.verify(&expr);
    assert!(result.is_ok(), "true ↔ true should be valid");
}

#[test]
#[cfg(feature = "verification")]
fn z3_iff_detects_inequivalence() {
    use logicaffeine_verify::{VerificationSession, VerifyExpr};
    let session = VerificationSession::new();
    // true ↔ false should be invalid
    let expr = VerifyExpr::iff(VerifyExpr::Bool(true), VerifyExpr::Bool(false));
    let result = session.verify(&expr);
    assert!(result.is_err(), "true ↔ false should be invalid");
}

#[test]
#[cfg(feature = "verification")]
fn z3_bmc_verify_temporal_safety_holds() {
    use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyOp};
    let session = VerificationSession::new();
    // Simple FSM: s starts at 0, increments each step
    // Property: s >= 0 (should hold for all steps)
    let initial = VerifyExpr::eq(VerifyExpr::var("s"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("s_next"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("s"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("s"), VerifyExpr::int(0));
    let result = session.verify_temporal(&initial, &transition, &property, 5);
    assert!(result.is_ok(), "s >= 0 should hold for 5 steps. Error: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn z3_bmc_catches_violation() {
    use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyOp};
    let session = VerificationSession::new();
    let initial = VerifyExpr::eq(VerifyExpr::var("s"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("s_next"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("s"), VerifyExpr::int(1)),
    );
    // Property: s < 3 — will be violated when s reaches 3
    let property = VerifyExpr::lt(VerifyExpr::var("s"), VerifyExpr::int(3));
    let result = session.verify_temporal(&initial, &transition, &property, 5);
    assert!(result.is_err(), "s reaches 3 at step 3 — s < 3 should be violated");
}

// ═══════════════════════════════════════════════════════════════════════════
// RENAME_VAR_IN_EXPR — DIAMOND TESTS (all 17 variants must recurse)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn rename_var_handles_apply() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::apply("P", vec![VerifyExpr::var("s"), VerifyExpr::var("x")]);
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::Apply { args, .. } => {
            assert!(matches!(&args[0], VerifyExpr::Var(n) if n == "s_0"),
                "Apply arg 0 must be renamed. Got: {:?}", renamed);
            assert!(matches!(&args[1], VerifyExpr::Var(n) if n == "x"),
                "Apply arg 1 must be unchanged. Got: {:?}", renamed);
        }
        _ => panic!("Must stay Apply. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_forall() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("s"), VerifyExpr::var("x")),
    );
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::ForAll { body, .. } => match body.as_ref() {
            VerifyExpr::Binary { left, .. } => {
                assert!(matches!(left.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                    "ForAll body var must be renamed. Got: {:?}", renamed);
            }
            _ => panic!("Body must be Binary. Got: {:?}", body),
        },
        _ => panic!("Must stay ForAll. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_exists() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::exists(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::lt(VerifyExpr::var("s"), VerifyExpr::var("x")),
    );
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::Exists { body, .. } => match body.as_ref() {
            VerifyExpr::Binary { left, .. } => {
                assert!(matches!(left.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                    "Exists body var must be renamed. Got: {:?}", renamed);
            }
            _ => panic!("Body must be Binary. Got: {:?}", body),
        },
        _ => panic!("Must stay Exists. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_bitvec_binary() {
    use logicaffeine_verify::{rename_var_in_expr, BitVecOp};
    let expr = VerifyExpr::bv_binary(BitVecOp::Add, VerifyExpr::var("s"), VerifyExpr::bv_const(8, 1));
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::BitVecBinary { left, .. } => {
            assert!(matches!(left.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                "BitVecBinary left must be renamed. Got: {:?}", renamed);
        }
        _ => panic!("Must stay BitVecBinary. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_select() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::Select {
        array: Box::new(VerifyExpr::var("mem")),
        index: Box::new(VerifyExpr::var("s")),
    };
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::Select { index, .. } => {
            assert!(matches!(index.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                "Select index must be renamed. Got: {:?}", renamed);
        }
        _ => panic!("Must stay Select. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_store() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::Store {
        array: Box::new(VerifyExpr::var("mem")),
        index: Box::new(VerifyExpr::var("s")),
        value: Box::new(VerifyExpr::var("v")),
    };
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::Store { index, .. } => {
            assert!(matches!(index.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                "Store index must be renamed. Got: {:?}", renamed);
        }
        _ => panic!("Must stay Store. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_bitvec_extract() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::BitVecExtract {
        high: 7, low: 4,
        operand: Box::new(VerifyExpr::var("s")),
    };
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::BitVecExtract { operand, .. } => {
            assert!(matches!(operand.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                "Extract operand must be renamed. Got: {:?}", renamed);
        }
        _ => panic!("Must stay BitVecExtract. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_handles_bitvec_concat() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::BitVecConcat(
        Box::new(VerifyExpr::var("s")),
        Box::new(VerifyExpr::var("t")),
    );
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    match &renamed {
        VerifyExpr::BitVecConcat(l, _) => {
            assert!(matches!(l.as_ref(), VerifyExpr::Var(n) if n == "s_0"),
                "Concat left must be renamed. Got: {:?}", renamed);
        }
        _ => panic!("Must stay BitVecConcat. Got: {:?}", renamed),
    }
}

#[test]
fn rename_var_preserves_literals() {
    use logicaffeine_verify::rename_var_in_expr;
    assert_eq!(rename_var_in_expr(&VerifyExpr::Int(42), "s", "s_0"), VerifyExpr::Int(42));
    assert_eq!(rename_var_in_expr(&VerifyExpr::Bool(true), "s", "s_0"), VerifyExpr::Bool(true));
    let bv = VerifyExpr::bv_const(8, 0xFF);
    assert_eq!(rename_var_in_expr(&bv, "s", "s_0"), VerifyExpr::bv_const(8, 0xFF));
}

#[test]
fn rename_var_does_not_rename_non_matching() {
    use logicaffeine_verify::rename_var_in_expr;
    let expr = VerifyExpr::var("x");
    let renamed = rename_var_in_expr(&expr, "s", "s_0");
    assert_eq!(renamed, VerifyExpr::var("x"), "Non-matching var must stay unchanged");
}

// ═══════════════════════════════════════════════════════════════════════════
// DIAMOND BITVECTOR Z3 TESTS — prove the encoder actually works
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_bitvec_diamond {
    use logicaffeine_verify::{BitVecOp, VerificationSession, VerifyExpr, VerifyType};

    #[test]
    fn z3_bv_and_computes_correctly() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 0xFF)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 0x0F)));
        let result = VerifyExpr::bv_binary(BitVecOp::And, VerifyExpr::var("a"), VerifyExpr::var("b"));
        let check = VerifyExpr::eq(result, VerifyExpr::bv_const(8, 0x0F));
        assert!(s.verify(&check).is_ok(), "0xFF & 0x0F must equal 0x0F");
    }

    #[test]
    fn z3_bv_or_computes_correctly() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 0xF0)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 0x0F)));
        let result = VerifyExpr::bv_binary(BitVecOp::Or, VerifyExpr::var("a"), VerifyExpr::var("b"));
        let check = VerifyExpr::eq(result, VerifyExpr::bv_const(8, 0xFF));
        assert!(s.verify(&check).is_ok(), "0xF0 | 0x0F must equal 0xFF");
    }

    #[test]
    fn z3_bv_xor_self_is_zero() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 0xFF)));
        let result = VerifyExpr::bv_binary(BitVecOp::Xor, VerifyExpr::var("a"), VerifyExpr::var("a"));
        let check = VerifyExpr::eq(result, VerifyExpr::bv_const(8, 0x00));
        assert!(s.verify(&check).is_ok(), "0xFF ^ 0xFF must equal 0x00");
    }

    #[test]
    fn z3_bv_add_no_overflow() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 100)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 28)));
        let result = VerifyExpr::bv_binary(BitVecOp::Add, VerifyExpr::var("a"), VerifyExpr::var("b"));
        let check = VerifyExpr::eq(result, VerifyExpr::bv_const(8, 128));
        assert!(s.verify(&check).is_ok(), "100 + 28 must equal 128");
    }

    #[test]
    fn z3_bv_add_overflow_wraps() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 200)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 100)));
        let result = VerifyExpr::bv_binary(BitVecOp::Add, VerifyExpr::var("a"), VerifyExpr::var("b"));
        // 300 mod 256 = 44
        let check = VerifyExpr::eq(result, VerifyExpr::bv_const(8, 44));
        assert!(s.verify(&check).is_ok(), "200 + 100 must wrap to 44 in 8-bit");
    }

    #[test]
    fn z3_bv_shl_computes_correctly() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 1)));
        let result = VerifyExpr::bv_binary(BitVecOp::Shl, VerifyExpr::var("a"), VerifyExpr::bv_const(8, 4));
        let check = VerifyExpr::eq(result, VerifyExpr::bv_const(8, 16));
        assert!(s.verify(&check).is_ok(), "1 << 4 must equal 16");
    }

    #[test]
    fn z3_bv_ult_true_case() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 5)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 10)));
        let cmp = VerifyExpr::bv_binary(BitVecOp::ULt, VerifyExpr::var("a"), VerifyExpr::var("b"));
        assert!(s.verify(&cmp).is_ok(), "5 <u 10 must be true");
    }

    #[test]
    fn z3_bv_ult_unsigned_255_not_less_than_1() {
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 255)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 1)));
        let cmp = VerifyExpr::bv_binary(BitVecOp::ULt, VerifyExpr::var("a"), VerifyExpr::var("b"));
        assert!(s.verify(&cmp).is_err(), "255 <u 1 must be false (unsigned)");
    }

    #[test]
    fn z3_bv_slt_signed_minus1_less_than_1() {
        // 255 as signed 8-bit is -1. -1 <s 1 is TRUE.
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.declare("b", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 255)));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("b"), VerifyExpr::bv_const(8, 1)));
        let cmp = VerifyExpr::bv_binary(BitVecOp::SLt, VerifyExpr::var("a"), VerifyExpr::var("b"));
        assert!(s.verify(&cmp).is_ok(), "255 (=-1 signed) <s 1 must be true");
    }

    #[test]
    fn z3_bv_extract_low_nibble() {
        // Extract bits [3:0] from 0xAB → 0x0B
        let mut s = VerificationSession::new();
        s.declare("a", VerifyType::BitVector(8));
        s.assume(&VerifyExpr::eq(VerifyExpr::var("a"), VerifyExpr::bv_const(8, 0xAB)));
        let extracted = VerifyExpr::BitVecExtract {
            high: 3, low: 0,
            operand: Box::new(VerifyExpr::var("a")),
        };
        let check = VerifyExpr::eq(extracted, VerifyExpr::bv_const(4, 0x0B));
        assert!(s.verify(&check).is_ok(), "Extract [3:0] from 0xAB must be 0x0B");
    }

    #[test]
    fn z3_bv_concat_joins_nibbles() {
        // Concat 0xA (4-bit) with 0xB (4-bit) → 0xAB (8-bit)
        let s = VerificationSession::new();
        let hi = VerifyExpr::bv_const(4, 0xA);
        let lo = VerifyExpr::bv_const(4, 0xB);
        let joined = VerifyExpr::BitVecConcat(Box::new(hi), Box::new(lo));
        let check = VerifyExpr::eq(joined, VerifyExpr::bv_const(8, 0xAB));
        assert!(s.verify(&check).is_ok(), "Concat 0xA:0xB must be 0xAB");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DIAMOND ARRAY Z3 TESTS — prove array theory works
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_array_diamond {
    use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};

    #[test]
    fn z3_array_store_then_select_retrieves_value() {
        let mut s = VerificationSession::new();
        s.declare("mem", VerifyType::Array(
            Box::new(VerifyType::Int),
            Box::new(VerifyType::Int),
        ));
        let stored = VerifyExpr::Store {
            array: Box::new(VerifyExpr::var("mem")),
            index: Box::new(VerifyExpr::Int(0)),
            value: Box::new(VerifyExpr::Int(42)),
        };
        let selected = VerifyExpr::Select {
            array: Box::new(stored),
            index: Box::new(VerifyExpr::Int(0)),
        };
        let check = VerifyExpr::eq(selected, VerifyExpr::Int(42));
        assert!(s.verify(&check).is_ok(), "Store then Select at same index must return 42");
    }

    #[test]
    fn z3_array_select_different_index_is_independent() {
        let mut s = VerificationSession::new();
        s.declare("mem", VerifyType::Array(
            Box::new(VerifyType::Int),
            Box::new(VerifyType::Int),
        ));
        let stored = VerifyExpr::Store {
            array: Box::new(VerifyExpr::var("mem")),
            index: Box::new(VerifyExpr::Int(0)),
            value: Box::new(VerifyExpr::Int(42)),
        };
        let selected = VerifyExpr::Select {
            array: Box::new(stored),
            index: Box::new(VerifyExpr::Int(1)),
        };
        let check = VerifyExpr::eq(selected, VerifyExpr::Int(42));
        assert!(s.verify(&check).is_err(),
            "Store at index 0, select at index 1 — value should be unknown, not provably 42");
    }
}
