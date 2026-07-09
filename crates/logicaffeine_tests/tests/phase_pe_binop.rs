//! Phase B1 — evalBinOp parity + identities (PE_IMPROVE §5, closes gap G2).
//!
//! The self-interpreter's `applyBinOp` (phase_futamura.rs) is the spec: the PE must fold
//! a static binop to the same value the interpreter computes, and on the *undefined* cases
//! (div-by-zero, type-mismatch, ops it cannot represent) it must **residualize, not fold to
//! garbage**. RED-first per CLAUDE.md.

mod pe_support;

use pe_support::*;

// ===========================================================================
// B1.0 — Residualize-on-undefined (soundness precondition).
//
// peExpr's CBinOp fold does `valToExpr(evalBinOp(...))` unconditionally; when evalBinOp
// returns VNothing (div-by-zero, type-mismatch, or any op it doesn't support) valToExpr
// yields a CVar "__unresolvable" — a miscompile. The PE must residualize instead.
// ===========================================================================

/// An operation the PE's evalBinOp does not yet fold (Float multiply) but the interpreter
/// executes: the residual must run identically to the original, never `__unresolvable`.
/// (After B1.2 this also folds; here we only require correctness via residualization.)
#[test]
fn unsupported_op_residualizes_not_garbage() {
    let program = "## Main\nLet a be 5.0.\nLet b be 2.0.\nShow a * b.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "residual must not contain __unresolvable garbage:\n{}",
        residual
    );
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert!(tw.is_value(), "tree-walk should produce a value: {:?}", tw);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

/// Division by zero with static operands must residualize (so the runtime reproduces the
/// interpreter's error) — never fold to `__unresolvable`.
#[test]
fn div_by_zero_residualizes() {
    let program = "## Main\nLet a be 7.\nLet b be 0.\nShow a / b.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "div-by-zero must residualize, not fold to garbage:\n{}",
        residual
    );
    // Both the original and the residual error identically (div by zero) under the
    // tree-walker — the PE preserved meaning.
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

/// Modulo by zero, same requirement.
#[test]
fn mod_by_zero_residualizes() {
    let program = "## Main\nLet a be 7.\nLet b be 0.\nShow a % b.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "mod-by-zero must residualize:\n{}",
        residual
    );
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

/// A binop the spec leaves undefined (Int on the left of a Text concat) must residualize,
/// matching the interpreter rather than fabricating a value.
#[test]
fn int_plus_text_residualizes() {
    let program = "## Main\nLet a be 5.\nShow a + \"x\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "Int+Text must residualize:\n{}",
        residual
    );
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

/// A still-supported, fully-static binop continues to fold (no regression from the
/// residualize guard).
#[test]
fn supported_static_op_still_folds() {
    let program = "## Main\nLet a be 6.\nLet b be 7.\nShow a * b.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("42"),
        "static multiply should fold to 42:\n{}",
        residual
    );
    assert!(
        !residual.contains(" * "),
        "folded residual should not contain the multiply operator:\n{}",
        residual
    );
    assert_run_equals(program, "42");
}

// ===========================================================================
// B1.1 — Int op parity: xor (^), shifts (<< >>), wrapping overflow.
// Surface syntax: `a xor b`, `a shifted left by b`, `a shifted right by b`.
// Spec: applyBinOp VInt+VInt (phase_futamura.rs:255-260).
// ===========================================================================

/// Folds to the literal result and leaves no operator in the residual.
fn assert_folds_to(program: &str, expected: &str, banned_op: &str) {
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains(expected),
        "expected folded literal {} in residual:\n{}",
        expected,
        residual
    );
    assert!(
        !residual.contains(banned_op),
        "residual should be folded (no `{}`):\n{}",
        banned_op,
        residual
    );
    assert_run_equals(program, expected);
}

#[test]
fn int_xor_folds() {
    // 12 ^ 10 = 6
    assert_folds_to("## Main\nLet a be 12.\nLet b be 10.\nShow a xor b.", "6", " xor ");
}

#[test]
fn int_shl_folds() {
    // 5 << 2 = 20
    assert_folds_to(
        "## Main\nLet a be 5.\nLet b be 2.\nShow a shifted left by b.",
        "20",
        "shifted left",
    );
}

#[test]
fn int_shr_folds() {
    // 20 >> 2 = 5
    assert_folds_to(
        "## Main\nLet a be 20.\nLet b be 2.\nShow a shifted right by b.",
        "5",
        "shifted right",
    );
}

// NOTE: Int overflow semantics (e.g. i64::MAX + 1) are intentionally NOT tested here.
// Neither the tree-walker (interpreter.rs::apply_add) nor codegen uses wrapping/checked
// arithmetic — both emit plain Rust `+`, which panics in debug and wraps in release. That
// is an undecided language-semantics policy (wrap vs checked-error vs leave-as-is), flagged
// for a deliberate decision before the PE can correctly fold overflowing constants.

/// Static shifts inside a fully-unrolled static loop fold away entirely — and crucially the
/// decompiler never emits raw `<<` (its `opToStr` now renders shift/xor as surface syntax,
/// so even a residualized shift would re-parse).
#[test]
fn static_shift_in_loop_folds() {
    let program = "## Main\nLet mutable acc be 0.\nRepeat for n from 1 to 3:\n    Set acc to acc + (n shifted left by 1).\nShow acc.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains(" << ") && !residual.contains("__unresolvable"),
        "residual must not contain raw `<<` or garbage:\n{}",
        residual
    );
    // 2*(1+2+3) = 12
    assert_run_equals(program, "12");
}

// ===========================================================================
// B1.2 — Float parity: VFloat+VFloat, VInt+VFloat, VFloat+VInt.
// Spec: applyBinOp (phase_futamura.rs:262-339). valToExpr already handles VFloat.
// Float display strings are not hardcoded — correctness is checked behaviorally
// (PE residual runs identically to the tree-walker) plus the fold is structural.
// ===========================================================================

/// The static binop folded (no operator remains) AND the residual runs identically to the
/// original under the tree-walker, with no garbage. Avoids depending on float formatting.
fn assert_folds_behaviorally(program: &str, banned_op: &str) {
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "residual must not contain garbage:\n{}",
        residual
    );
    assert!(
        !residual.contains(banned_op),
        "static op should have folded (no `{}`):\n{}",
        banned_op,
        residual
    );
    let tw = run_treewalk(program);
    assert!(tw.is_value(), "tree-walk should produce a value: {:?}", tw);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

#[test]
fn float_add_folds() {
    assert_folds_behaviorally("## Main\nLet a be 5.0.\nLet b be 2.5.\nShow a + b.", " + ");
}

#[test]
fn float_sub_mul_div_fold() {
    assert_folds_behaviorally("## Main\nLet a be 9.0.\nLet b be 2.0.\nShow a - b.", " - ");
    assert_folds_behaviorally("## Main\nLet a be 1.5.\nLet b be 4.0.\nShow a * b.", " * ");
    assert_folds_behaviorally("## Main\nLet a be 9.0.\nLet b be 2.0.\nShow a / b.", " / ");
}

#[test]
fn float_comparisons_fold() {
    assert_run_equals("## Main\nLet a be 5.0.\nLet b be 6.0.\nShow a < b.", "true");
    assert_run_equals("## Main\nLet a be 5.0.\nLet b be 6.0.\nShow a > b.", "false");
    assert_run_equals("## Main\nLet a be 5.0.\nLet b be 5.0.\nShow a == b.", "true");
    assert_run_equals("## Main\nLet a be 5.0.\nLet b be 6.0.\nShow a != b.", "true");
}

#[test]
fn mixed_int_float_folds() {
    // Int + Float and Float + Int both promote to Float; fold and run identically.
    assert_folds_behaviorally("## Main\nLet a be 5.\nLet b be 2.5.\nShow a + b.", " + ");
    assert_folds_behaviorally("## Main\nLet a be 5.0.\nLet b be 2.\nShow a + b.", " + ");
}

#[test]
fn float_div_by_zero_residualizes() {
    let program = "## Main\nLet a be 5.0.\nLet b be 0.0.\nShow a / b.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "float div-by-zero must residualize, not garbage:\n{}",
        residual
    );
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

/// Float round-trip fidelity: a folded float must decompile to a literal that re-parses to
/// the *same* value (not silently truncated to an Int or losing precision).
#[test]
fn float_fold_roundtrip_fidelity() {
    // 0.1 + 0.2 has a non-trivial binary representation; the folded literal must reproduce
    // exactly what the interpreter prints for the same computation.
    assert_folds_behaviorally("## Main\nLet a be 0.1.\nLet b be 0.2.\nShow a + b.", " + ");
    // A whole-valued float result must stay a Float (e.g. print like the interpreter does),
    // verified behaviorally against the tree-walker.
    assert_folds_behaviorally("## Main\nLet a be 2.0.\nLet b be 3.0.\nShow a * b.", " * ");
}

// ===========================================================================
// B1.3 — Text concatenation coercion: VText+VInt, VText+VBool.
// Spec: applyBinOp (phase_futamura.rs:358-384). Note the spec does NOT define
// VInt+VText (Int on the left) — that must residualize (covered by B1.0).
// ===========================================================================

#[test]
fn text_plus_int_folds() {
    let program = "## Main\nLet a be 5.\nShow \"n=\" + a.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(residual.contains("n=5"), "should fold to \"n=5\":\n{}", residual);
    assert_run_equals(program, "n=5");
}

#[test]
fn text_plus_bool_folds() {
    assert_run_equals("## Main\nLet a be true.\nShow \"f=\" + a.", "f=true");
    assert_run_equals("## Main\nLet a be false.\nShow \"f=\" + a.", "f=false");
}

/// Int on the left of a Text concat is undefined per the spec → must residualize, not
/// fabricate a value (already exercised by B1.0's int_plus_text_residualizes; reaffirmed
/// here for the coverage matrix).
#[test]
fn int_plus_text_still_residualizes() {
    let program = "## Main\nLet a be 5.\nShow a + \"x\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "Int+Text must residualize:\n{}",
        residual
    );
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

// ===========================================================================
// B1.4 — Type-mismatch residualizes correctly (and a note on temporal).
//
// The self-interpreter leaves cross-type binops undefined (VNothing); the PE must
// residualize them (never fabricate a value), so the residual reproduces the interpreter's
// runtime error exactly.
//
// Temporal-result folding (Date+Duration, Date-Date, …) is NOT reachable here: the IR has
// no Date/Moment literal CExpr and no surface syntax for static dates (`today`/`now` are
// runtime values), so such ops can never have two static operands to fold. Residualization
// — guaranteed by B1.0 — is the sound and only behavior.
// ===========================================================================

/// Assert a binop residualizes (no garbage) and the residual behaves exactly like the
/// original under the tree-walker — including reproducing a runtime type error.
fn assert_residualizes_same_behavior(program: &str) {
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "must residualize cleanly, no garbage:\n{}",
        residual
    );
    let tw = run_treewalk(program);
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

#[test]
fn bool_compared_to_int_residualizes() {
    assert_residualizes_same_behavior("## Main\nLet a be true.\nLet b be 5.\nShow a < b.");
}

#[test]
fn text_compared_to_int_residualizes() {
    assert_residualizes_same_behavior("## Main\nLet a be \"x\".\nLet b be 5.\nShow a < b.");
}

#[test]
fn bool_added_to_int_residualizes() {
    assert_residualizes_same_behavior("## Main\nLet a be true.\nLet b be 5.\nShow a + b.");
}

#[test]
fn and_with_int_residualizes() {
    assert_residualizes_same_behavior("## Main\nLet a be true.\nLet b be 5.\nShow a and b.");
}
